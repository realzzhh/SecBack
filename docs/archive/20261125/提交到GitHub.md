# 提交到 GitHub

本地已经完成 `git init` 和首次提交（分支 `main`），只差推到远端。

## 1. 先在 GitHub 建空仓库

网页上 **New repository**：

- 名字随意，比如 `secback`
- **建议选 Private**（这是密码本工具）
- **不要**勾选 "Add a README file"、".gitignore"、"license"
  —— 勾了远端会先有一个提交，直接推会冲突，得先 pull 再处理。

## 2. 关联远端并推送

把 `<你的用户名>` 和 `<仓库名>` 换掉：

```bash
cd /d/WorkSpace/SecBackTauri

git remote add origin https://github.com/<你的用户名>/<仓库名>.git
git push -u origin main
```

## 3. 认证

**HTTPS 方式**：GitHub 不再接受账号密码。密码框里要填
Personal Access Token（Settings → Developer settings → Personal access tokens，
勾 `repo` 权限）。Windows 上通常会被凭据管理器记住，只输一次。

**SSH 方式**（推荐，配一次以后不用管）：

```bash
ssh-keygen -t ed25519 -C "your@email.com"     # 一路回车
cat ~/.ssh/id_ed25519.pub                      # 复制内容到 GitHub → SSH keys
ssh -T git@github.com                          # 看到 "Hi xxx" 就通了

git remote add origin git@github.com:<你的用户名>/<仓库名>.git
git push -u origin main
```

## 4. 之后日常提交

```bash
git add -A
git commit -m "说明这次改了什么"
git push
```

## 已经排除的内容

`target/`（6GB+）、`node_modules/`、`delivery/*.exe`、`*.properties`（密库）、
`src-tauri/gen/`（自动生成）。所以仓库只有约 2MB、38 个文件。

改完 `.gitignore` 想确认某类文件是否被挡：

```bash
git check-ignore -v 某个文件      # 显示命中了哪条规则
git status --short                # 看还有哪些没提交
```
