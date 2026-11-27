use serde_json::Value;
pub const MAX_MESSAGE_BYTES: usize = 12 * 1024 * 1024;

pub fn parse_id(s: &str) -> Option<[u8;16]> {
    if s.len()!=32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    let mut id=[0;16];
    for (i,pair) in s.as_bytes().chunks_exact(2).enumerate() {
        let digit=|b:u8| if b.is_ascii_digit(){b-b'0'}else{b.to_ascii_lowercase()-b'a'+10};
        id[i]=digit(pair[0])*16+digit(pair[1]);
    }
    Some(id)
}
fn text(v:&Value, key:&str, max:usize) -> Result<(),String> {
    match v.get(key).and_then(Value::as_str) {
        Some(s) if s.len()<=max => Ok(()),
        _ => Err(format!("字段 {key} 缺失、类型错误或过长")),
    }
}
fn id(v:&Value,key:&str)->Result<(),String>{
    if v[key].as_str().and_then(parse_id).is_some(){Ok(())}else{Err(format!("字段 {key} 不是有效项目标识"))}
}
fn slot(v:&Value,key:&str)->Result<(),String>{
    if v[key].as_u64().is_some_and(|n| n<3){Ok(())}else{Err("账户位置无效".into())}
}
pub fn validate(v:&Value)->Result<(),String>{
    if let Some(request)=v.get("request") {
        if !request.as_str().is_some_and(|s|s.len()<=128)
            && !request.as_u64().is_some_and(|n|n<=9_007_199_254_740_991) {
            return Err("请求标识无效".into());
        }
    }
    let t=v["t"].as_str().ok_or("缺少命令类型")?;
    match t {
        "ready"|"touch"|"quit"|"about"|"lock"|"background"|"import"|"export"|"import_cancel"|"up"|"down"|"recover_cancel"=>{},
        "theme_sync"=>{ if !v["dark"].is_boolean(){return Err("主题参数无效".into());} },
        "login"|"import_unlock"|"set_recovery"=>{text(v,"pwd",4096)?;if let Some(c)=v.get("confirm"){if !c.is_string()||c.as_str().unwrap().len()>4096{return Err("确认密码无效".into());}}},
        "rekey"=>{text(v,"old",4096)?;text(v,"new",4096)?;},
        "recover"=>text(v,"code",256)?,
        "recover_set_password"=>text(v,"new",4096)?,
        "import_path"=>text(v,"path",32768)?,
        "select"|"delete"=>id(v,"id")?,
        "reorder"=>{id(v,"id")?;if !v["to"].as_u64().is_some_and(|n|n<100_000){return Err("排序位置无效".into());}},
        "copy"|"reveal"=>{id(v,"id")?;slot(v,"slot")?;if t=="copy"&&!matches!(v["field"].as_str(),Some("user"|"pass")){return Err("复制字段无效".into());}},
        "create"|"save"=>{
            if t=="save"{id(v,"id")?;}
            for k in ["name","remark"]{text(v,k,secback_core::format::MAX_FIELD_BYTES)?;}
            for k in ["s0","s1","s2"]{if let Some(a)=v.get(k){
                for key in ["user","remark"]{text(a,key,secback_core::format::MAX_FIELD_BYTES)?;}
                if let Some(p)=a.get("pass"){if !p.is_null(){text(a,"pass",secback_core::format::MAX_FIELD_BYTES)?;}}
                if a.get("source").is_some(){slot(a,"source")?;}
            }}
        },
        "import_commit"|"import_plan"=>{
            for k in ["mergeSameName","skipProjects","useIncomingMetadata"]{
                if let Some(values)=v.get(k){let list=values.as_array().ok_or("导入参数须为列表")?;
                    if list.len()>100_000{return Err("导入列表过长".into());}
                    if list.iter().any(|x|x.as_str().and_then(parse_id).is_none()){return Err("导入项目标识无效".into());}
                }
            }
            if let Some(values)=v.get("excludeSlots"){let list=values.as_array().ok_or("账户参数须为列表")?;
                if list.len()>300_000{return Err("账户列表过长".into());}
                for a in list{id(a,"id")?;slot(a,"slot")?;}
            }
        },
        _=>return Err("未知命令".into()),
    }
    Ok(())
}
#[cfg(test)] mod tests {
 use super::*; use serde_json::json;
 #[test] fn unicode_ids_never_panic(){for id in ["éééééééééééééééé","中000000000000000000000000000000","ffffffffffffffffffffffffffffffff"]{let _=parse_id(id);}assert!(parse_id("éééééééééééééééé").is_none());}
 #[test] fn rejects_wraparound_slot(){assert!(validate(&json!({"t":"reveal","id":"00".repeat(16),"slot":256})).is_err());}
 #[test] fn rejects_unknown_and_wrong_types(){assert!(validate(&json!({"t":"oops"})).is_err());assert!(validate(&json!({"t":"save","id":"00".repeat(16),"name":7,"remark":""})).is_err());}
 #[test] fn validates_receipt_identifiers(){assert!(validate(&json!({"t":"import_plan","request":1})).is_ok());assert!(validate(&json!({"t":"ready","request":{"large":"object"}})).is_err());}
}
