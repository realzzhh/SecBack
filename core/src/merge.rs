//! 跨机器导入的合并算法。
//!
//! 场景：两台机器各有自己的密库，两份都不完整，需要取并集。
//! 全部做成纯函数——不改入参、不碰磁盘，可以反复调用（预览一次、确认前再算一次）。
//!
//! **幂等**是这个模块的核心约束：同一个文件反复导入，第二次起不再产生任何变化。
//! 靠的是溢出副本的 id 由 (对方项目 id, 组号) 确定性派生，而不是随机生成。

use crate::format::{Account, Project, ProjectId, MAX_SLOTS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeAction {
    Add,
    Merge,
    Skip,
}

/// 预览里"对方某个账户这次会落到哪里"。
#[derive(Debug, Clone)]
pub struct MergeSlot {
    pub acc: Account,
    /// 在【对方项目】里的槽位下标（取消勾选时要用它定位）
    pub src_slot: u8,
    /// 落进本机项目的哪一槽；-1 = 进副本
    pub local_slot: i32,
    /// 与本机已有账户完全重复，不会新增
    pub duplicate: bool,
    pub excluded: bool,
}

/// 预览里"一个对方项目对应本机的处置结果"。
#[derive(Debug, Clone)]
pub struct MergeEntry {
    pub action: MergeAction,
    pub incoming_id: ProjectId,
    pub name: String,
    /// 合并时本机项目的下标；新增为 -1
    pub local_index: i32,
    /// 槽位装不下而另存的副本条数
    pub copies: i32,
    /// 是否靠"同名"匹配上的（而非 id）
    pub same_name: bool,
    pub slots: Vec<MergeSlot>,
}

#[derive(Debug, Clone, Default)]
pub struct MergeOptions {
    /// 我方默认策略：疑似同名（id 不同、名称相同）默认合并，用户可逐条取消。
    pub merge_same_name: Vec<ProjectId>,
    /// 整条不要的对方项目。
    pub skip_projects: Vec<ProjectId>,
    /// 被取消勾选的账户。
    pub exclude_slots: Vec<(ProjectId, u8)>,
}

impl MergeOptions {
    pub fn is_slot_excluded(&self, pid: &ProjectId, slot: u8) -> bool {
        self.exclude_slots
            .iter()
            .any(|(id, s)| *s == slot && id == pid)
    }
}

#[derive(Debug, Clone, Default)]
pub struct MergeResult {
    /// 合并后的完整列表，可直接写盘
    pub merged: Vec<Project>,
    pub entries: Vec<MergeEntry>,

    pub added: i32,
    pub merged_count: i32,
    pub skipped: i32,
    /// 溢出副本条数
    pub overflow_count: i32,
    /// 实际新增的账户数
    pub slots_added: i32,
}

/// 由 base + 组号确定性地派生一个副本 id。
///
/// 唯一重要的是【确定性】：同一个对方项目 + 同一个组号永远得到同一个 id。
/// 换成随机赋值的话，同一个文件每导一次就多一批副本，库会越导越臃肿。
pub fn derive_copy_id(base: &ProjectId, ordinal: u32) -> ProjectId {
    const FNV_OFFSET: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;

    let mut h1 = FNV_OFFSET;
    for b in base {
        h1 ^= *b as u64;
        h1 = h1.wrapping_mul(FNV_PRIME);
    }
    for i in 0..4 {
        h1 ^= ((ordinal >> (8 * i)) & 0xFF) as u64;
        h1 = h1.wrapping_mul(FNV_PRIME);
    }

    let mut h2 = h1 ^ 0x9E3779B97F4A7C15;
    for i in 0..8 {
        h2 ^= (h1 >> (8 * i)) & 0xFF;
        h2 = h2.wrapping_mul(FNV_PRIME);
    }

    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&h1.to_le_bytes());
    out[8..].copy_from_slice(&h2.to_le_bytes());
    out
}

/// 只用于"同名匹配"的比较。全角空格（U+3000）也要算空白——
/// 中文输入法下很容易打出来，用户看不出区别，但字节完全不同。
fn trim_name(s: &str) -> &str {
    s.trim_matches(|c: char| {
        c == ' ' || c == '\t' || c == '\r' || c == '\n' || c == '\u{3000}'
    })
}

fn has_account(p: &Project, a: &Account) -> bool {
    for i in 0..MAX_SLOTS {
        if !p.slot_empty(i) && p.slots[i] == *a {
            return true;
        }
    }
    false
}

fn first_empty_slot(p: &Project) -> Option<usize> {
    (0..MAX_SLOTS).find(|i| p.slot_empty(*i))
}

pub fn merge(local: &[Project], incoming: &[Project], opt: &MergeOptions) -> MergeResult {
    let mut r = MergeResult {
        merged: local.to_vec(),
        ..Default::default()
    };

    // 只有【原有项目】能被匹配到。本次导入刚新增的项目不能当匹配目标，
    // 否则对方两个同名项目会被揉成一条，而它们本来是两条不同的记录。
    let base_count = local.len();

    for src in incoming {
        let mut e = MergeEntry {
            action: MergeAction::Add,
            incoming_id: src.id,
            name: src.name.clone(),
            local_index: -1,
            copies: 0,
            same_name: false,
            slots: Vec::new(),
        };

        // ---- 1. 找到本机对应项目 ----
        let mut local_idx: Option<usize> = (0..base_count).find(|i| local[*i].id == src.id);

        if local_idx.is_none() && opt.merge_same_name.contains(&src.id) {
            let want = trim_name(&src.name);
            if !want.is_empty() {
                if let Some(i) = (0..base_count).find(|i| trim_name(&local[*i].name) == want) {
                    local_idx = Some(i);
                    e.same_name = true;
                }
            }
        }

        // ---- 2. 收集对方的非空账户（被排除的也要留着，预览要显示） ----
        for s in 0..MAX_SLOTS {
            if src.slot_empty(s) {
                continue;
            }
            e.slots.push(MergeSlot {
                acc: src.slots[s].clone(),
                src_slot: s as u8,
                local_slot: -1,
                duplicate: false,
                excluded: opt.is_slot_excluded(&src.id, s as u8),
            });
        }

        // ---- 3. 整条跳过 ----
        if opt.skip_projects.contains(&src.id) {
            e.action = MergeAction::Skip;
            for ms in e.slots.iter_mut() {
                ms.excluded = true;
            }
            r.entries.push(e);
            r.skipped += 1;
            continue;
        }

        // ---- 4. 决定落点 ----
        let (dst_idx, is_new) = match local_idx {
            Some(i) => (i, false),
            None => {
                let mut p = src.clone();
                for s in 0..MAX_SLOTS {
                    p.slots[s] = Account::empty();
                }
                r.merged.push(p);
                (r.merged.len() - 1, true)
            }
        };
        if !is_new {
            e.local_index = dst_idx as i32;
        }

        // ---- 5. 逐账户落位：先判重，再找空槽 ----
        let mut overflow: Vec<Account> = Vec::new();
        for ms in e.slots.iter_mut() {
            if ms.excluded {
                continue;
            }
            if has_account(&r.merged[dst_idx], &ms.acc) {
                ms.duplicate = true;
                continue;
            }
            match first_empty_slot(&r.merged[dst_idx]) {
                Some(slot) => {
                    r.merged[dst_idx].slots[slot] = ms.acc.clone();
                    ms.local_slot = slot as i32;
                    r.slots_added += 1;
                }
                None => overflow.push(ms.acc.clone()),
            }
        }

        // ---- 6. 溢出的每 3 个装一条副本 ----
        // 副本 id 由 (对方项目 id, 组号) 确定性派生，所以同一个文件导入
        // 第二次会落到同一条副本上，里面的账户已存在 → 结果幂等。
        let mut group: u32 = 1;
        // 先在全部既有副本判重，避免某个早期副本被腾空后重复填入后期副本已有账户。
        overflow.retain(|acc| {
            let mut n = 1;
            loop {
                let cid = derive_copy_id(&src.id, n);
                match r.merged.iter().find(|p| p.id == cid) {
                    Some(p) if has_account(p, acc) => return false,
                    Some(_) => n += 1,
                    None => return true,
                }
            }
        });
        let mut i = 0;
        while i < overflow.len() {
            let cid = derive_copy_id(&src.id, group);

            let cp_idx = match r.merged.iter().position(|p| p.id == cid) {
                Some(idx) => idx,
                None => {
                    let mut cp = Project::new_with_id(cid);
                    cp.name = format!("{}（导入）", src.name);
                    if group > 1 {
                        cp.name.push_str(&group.to_string());
                    }
                    cp.remark = src.remark.clone();
                    cp.created_utc = src.created_utc;
                    cp.modified_utc = src.modified_utc;
                    r.merged.push(cp);
                    r.added += 1;
                    r.overflow_count += 1;
                    e.copies += 1;
                    r.merged.len() - 1
                }
            };

            // 只有判重或真正插入才消费账户；满副本继续尝试下一组。
            while i < overflow.len() {
                if has_account(&r.merged[cp_idx], &overflow[i]) {
                    i += 1;
                } else if let Some(slot) = first_empty_slot(&r.merged[cp_idx]) {
                    r.merged[cp_idx].slots[slot] = overflow[i].clone();
                    r.slots_added += 1;
                    i += 1;
                } else {
                    break;
                }
            }

            group += 1;
        }

        if is_new {
            r.added += 1;
        } else {
            r.merged_count += 1;
        }
        e.action = if is_new { MergeAction::Add } else { MergeAction::Merge };
        r.entries.push(e);
    }

    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_survives_full_or_edited_existing_copies() {
        for free in [0, 1, 2] {
            let mut local = proj(7, "原项目");
            let mut incoming = local.clone();
            for i in 0..MAX_SLOTS {
                local.slots[i] = acc(&format!("本机{i}"), "old");
                incoming.slots[i] = acc(&format!("外部{i}"), "new");
            }
            let mut copy = Project::new_with_id(derive_copy_id(&incoming.id, 1));
            copy.name = "用户改过的副本".into();
            for i in 0..MAX_SLOTS - free { copy.slots[i] = acc(&format!("副本{i}"), "keep"); }
            let first = merge(&[local, copy], &[incoming.clone()], &MergeOptions::default());
            for account in &incoming.slots {
                assert!(first.merged.iter().any(|p| p.slots.contains(account)));
            }
            assert_eq!(first.slots_added, 3);
            let second = merge(&first.merged, &[incoming.clone()], &MergeOptions::default());
            assert_eq!(second.slots_added, 0);
            assert_eq!(second.added, 0);
            // 早期副本腾出空位也不能把后期副本里的账户再复制一遍。
            let mut edited = first.merged;
            edited[1].slots[0] = Account::empty();
            let third = merge(&edited, &[incoming], &MergeOptions::default());
            assert_eq!(third.slots_added, 0);
            assert_eq!(third.added, 0);
        }
    }

    fn proj(id_byte: u8, name: &str) -> Project {
        let mut p = Project::new_with_id([id_byte; 16]);
        p.name = name.to_string();
        p
    }

    fn acc(user: &str, pass: &str) -> Account {
        Account {
            user: user.into(),
            pass: pass.into(),
            remark: String::new(),
        }
    }

    #[test]
    fn new_project_is_appended() {
        let local = vec![proj(1, "本机A")];
        let incoming = vec![proj(2, "对方B")];
        let r = merge(&local, &incoming, &MergeOptions::default());

        assert_eq!(r.merged.len(), 2);
        assert_eq!(r.added, 1);
        assert_eq!(r.entries[0].action, MergeAction::Add);
    }

    #[test]
    fn same_id_merges_into_existing() {
        let mut a = proj(1, "工作邮箱");
        a.slots[0] = acc("u1", "p1");

        let mut b = proj(1, "工作邮箱");
        b.slots[0] = acc("u2", "p2");

        let r = merge(&[a], &[b], &MergeOptions::default());
        assert_eq!(r.merged.len(), 1, "同 id 不该产生新项目");
        assert_eq!(r.merged[0].slots[1].user, "u2", "应落进第二个空槽");
        assert_eq!(r.slots_added, 1);
        assert_eq!(r.entries[0].action, MergeAction::Merge);
    }

    #[test]
    fn same_name_merged_only_when_asked() {
        let local = vec![proj(1, "工作邮箱")];
        let incoming = vec![proj(2, "工作邮箱")];

        // 默认不合并同名 → 两条
        let r1 = merge(&local, &incoming, &MergeOptions::default());
        assert_eq!(r1.merged.len(), 2);

        // 勾选了同名合并 → 一条
        let opt = MergeOptions {
            merge_same_name: vec![[2u8; 16]],
            ..Default::default()
        };
        let r2 = merge(&local, &incoming, &opt);
        assert_eq!(r2.merged.len(), 1);
        assert!(r2.entries[0].same_name);
    }

    #[test]
    fn duplicate_account_not_added_twice() {
        let mut a = proj(1, "X");
        a.slots[0] = acc("same", "same");
        let mut b = proj(1, "X");
        b.slots[0] = acc("same", "same");

        let r = merge(&[a], &[b], &MergeOptions::default());
        assert_eq!(r.slots_added, 0);
        assert!(r.entries[0].slots[0].duplicate);
    }

    #[test]
    fn overflow_becomes_copy() {
        let mut a = proj(1, "满的");
        for i in 0..3 {
            a.slots[i] = acc(&format!("u{}", i), "p");
        }
        let mut b = proj(1, "满的");
        b.slots[0] = acc("new1", "p");
        b.slots[1] = acc("new2", "p");

        let r = merge(&[a], &[b], &MergeOptions::default());
        assert_eq!(r.overflow_count, 1, "两个账户装不下，应生成一条副本");
        assert_eq!(r.merged.len(), 2);
        assert!(r.merged[1].name.ends_with("（导入）"));
        assert_eq!(r.merged[1].slots[0].user, "new1");
        assert_eq!(r.merged[1].slots[1].user, "new2");
    }

    /// 最重要的一条：同一个文件反复导入，第二次起必须零变化。
    #[test]
    fn merge_is_idempotent() {
        let mut a = proj(1, "满的");
        for i in 0..3 {
            a.slots[i] = acc(&format!("u{}", i), "p");
        }
        let mut b = proj(1, "满的");
        for i in 0..5 {
            let s = i % 3;
            if b.slot_empty(s) {
                b.slots[s] = acc(&format!("in{}", i), "p");
            }
        }

        let first = merge(&[a.clone()], &[b.clone()], &MergeOptions::default());
        assert!(first.overflow_count > 0, "这个用例要触发溢出才有意义");

        let second = merge(&first.merged, &[b.clone()], &MergeOptions::default());
        assert_eq!(second.merged.len(), first.merged.len(), "第二次不该多出项目");
        assert_eq!(second.slots_added, 0, "第二次不该新增账户");
        assert_eq!(second.added, 0);
        assert_eq!(second.overflow_count, 0);
    }

    #[test]
    fn skip_project_excludes_all_slots() {
        let local = vec![proj(1, "A")];
        let mut inc = proj(2, "B");
        inc.slots[0] = acc("u", "p");

        let opt = MergeOptions {
            skip_projects: vec![[2u8; 16]],
            ..Default::default()
        };
        let r = merge(&local, &[inc], &opt);
        assert_eq!(r.merged.len(), 1);
        assert_eq!(r.skipped, 1);
        assert_eq!(r.entries[0].action, MergeAction::Skip);
        assert!(r.entries[0].slots.iter().all(|s| s.excluded));
    }

    #[test]
    fn excluded_slot_not_added() {
        let local = vec![proj(1, "A")];
        let mut inc = proj(1, "A");
        inc.slots[0] = acc("u1", "p1");
        inc.slots[1] = acc("u2", "p2");

        let opt = MergeOptions {
            exclude_slots: vec![([1u8; 16], 0)],
            ..Default::default()
        };
        let r = merge(&local, &[inc], &opt);
        assert_eq!(r.slots_added, 1, "只应加第二个账户");
        assert_eq!(r.merged[0].slots[0].user, "u2");
    }

    #[test]
    fn new_project_slots_come_from_incoming() {
        let mut inc = proj(2, "B");
        inc.slots[0] = acc("u", "p");

        let r = merge(&[], &[inc], &MergeOptions::default());
        assert_eq!(r.merged.len(), 1);
        assert_eq!(r.merged[0].slots[0].user, "u");
        assert_eq!(r.slots_added, 1);
    }

    #[test]
    fn copy_id_is_deterministic() {
        let a = derive_copy_id(&[7u8; 16], 1);
        let b = derive_copy_id(&[7u8; 16], 1);
        let c = derive_copy_id(&[7u8; 16], 2);
        assert_eq!(a, b, "同样的输入必须给同样的 id");
        assert_ne!(a, c, "不同组号要给不同 id");
    }

    #[test]
    fn fullwidth_space_counts_as_blank_in_name_match() {
        let local = vec![proj(1, "工作邮箱")];
        let incoming = vec![proj(2, "\u{3000}工作邮箱 ")];

        let opt = MergeOptions {
            merge_same_name: vec![[2u8; 16]],
            ..Default::default()
        };
        let r = merge(&local, &incoming, &opt);
        assert_eq!(r.merged.len(), 1, "全角空格和半角空格都要能被裁掉");
    }
}
