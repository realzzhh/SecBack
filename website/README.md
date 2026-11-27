# SecBack 项目门户

独立的中文静态介绍页，当前介绍项目版本为 **0.1.1**。使用原生 HTML/CSS/JS，无构建依赖、远程字体、统计脚本或在线密库功能。不包含 APK / EXE，不影响 Tauri 内嵌的 `src/` 前端。

## 本地查看

在项目根目录执行：

```powershell
node website/serve.mjs
```

打开 `http://127.0.0.1:4173`。可通过 `PORT` 环境变量更换端口。服务仅监听本机。也可以直接打开 `website/index.html`，受浏览器本地文件策略影响时请使用预览服务。

## 结构与维护

- `index.html`：中文内容、版本、平台要求、下载链接、更新日志和 FAQ。
- `style.css`：墨绿 / 薄荷绿配色，深浅主题及桌面 / 手机布局。
- `app.js`：主题记忆、手机导航、平台预览与预览配色切换。
- `assets/`：项目 SVG 图标与仅含虚构账户的 0.1.1 应用截图。
- `serve.mjs`：零依赖本机预览服务器，不用于生产部署。

顶部主题切换影响网页；预览区的浅色 / 深色圆点单独切换桌面截图。移动端展示深色登录与导入预览截图。此处只有截图，不会读取本地密库或收集密码。所有页面内容与配图可离线加载。

## 下载与发布

Windows、Android 的下载按钮分别指向已发布的 0.1.1 附件，页脚和更新日志仍保留发行页入口：

```text
https://github.com/realzzhh/SecBack/releases/download/0.1.1/SecBack-0.1.1-Windows-x64.exe
https://github.com/realzzhh/SecBack/releases/download/0.1.1/SecBack-0.1.1-Android-arm64.apk
```

注意实际 Git 标签为 `0.1.1`，没有 `v` 前缀。发布新版本时，先将文件上传为 Release 附件并确认下载可用，再更新 `index.html` 中两处 `.download-link` 的版本标签和文件名，以及页面版本、日期和截图。地址中的标签和文件名必须与已发布附件完全一致；将二进制放入源码仓库或 Actions 构建产物列表不等于上传了 Release 附件。下载链接使用固定版本，避免页面仍写 0.1.1 时实际下载到其他版本。

可将 `website/` 的网页与 `assets/` 原样部署到任意静态托管目录。所有本地资源使用相对路径，支持 GitHub Pages 的仓库子路径；根页面为 `index.html`。`README.md` 和 `serve.mjs` 无需上传。用户已配置 GitHub Pages；提交网页修改后，需等待对应 Pages 工作流成功部署才会更新在线页面。

页面结构参考 InkMark 介绍页（`https://1atlas0.github.io/InkMark-Release/`）的平台入口、预览、功能和更新日志组织方式；未复制其代码、图标、截图或产品文案。SecBack 内容以本项目 README、能力与限制、0.1.1 界面与发布记录为依据。安全说明避免承诺绝对安全；Android 0.1.1 明确标注尚待真机验证。

## 验证记录

2026-11-27，本机浏览器完成以下检查：

- 320、393、768、1024、1440 像素宽度下无页面横向溢出，功能卡片按宽度呈现一列、两列和三列。
- 网页深浅主题、桌面截图浅色 / 深色切换、桌面 / 移动预览切换正常。
- 预览标签的方向键与 Home 切换、焦点与选中状态一致。
- 手机导航可展开，点击锚点后收起；FAQ 可展开。
- 可见图片加载成功，页内锚点目标存在，控制台无 error / warning。
- `node --check website/app.js`、`node --check website/serve.mjs` 和 Git 已跟踪改动的空白检查通过。

这次仅验证独立门户页面，未重新构建应用，也未进行手机真机测试。

同日下载链接修正：通过 GitHub 发行页核实标签及附件 URL，实际下载 EXE 和 APK，SHA256 与发行页展示值一致。此记录验证下载完整性，不代表新增应用运行或真机验证。
