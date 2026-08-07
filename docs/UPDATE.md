# WiParse 在线更新架构与服务器部署

本文档描述 WiParse 通过 HTTPS 服务器分发版本的架构、客户端行为，以及服务器侧部署要求。

## 1. 总体架构

```
┌─────────────────────┐         HTTPS GET          ┌──────────────────────────┐
│  WiParse.exe (GUI)  │ ─────────────────────────► │  update.your-domain.com  │
│  当前版本 1.0.1      │   /wiparse/stable/latest.json│  Nginx / OSS + CDN      │
└─────────┬───────────┘                              └────────────┬─────────────┘
          │                                                       │
          │ 发现新版本                                             │ 静态文件
          ▼                                                       ▼
┌─────────────────────┐         HTTPS GET          ┌──────────────────────────┐
│  下载 zip 到缓存     │ ◄───────────────────────── │  releases/1.0.2/*.zip    │
│  校验 SHA256        │                              │  + latest.json 指针      │
└─────────┬───────────┘                              └──────────────────────────┘
          │ 用户确认「安装并重启」
          ▼
┌─────────────────────┐
│ apply-update.ps1    │  等待主进程退出 → 解压覆盖 → 重启 WiParse.exe
└─────────────────────┘
```

### 设计原则

| 原则 | 说明 |
|------|------|
| **显示与数据分离** | 更新只替换程序文件；`config.json`、数据库、波形缓存不被覆盖 |
| **HTTPS 强制** | 客户端仅接受 `https://` 清单 URL，防止中间人篡改 |
| **完整性校验** | 每个安装包必须提供 SHA256；后续可扩展 Ed25519 签名 |
| **可回滚** | 安装脚本将旧 `WiParse.exe` 备份为 `WiParse.exe.bak` |
| **便携部署** | 无 MSI 安装器；适合当前 `dist/` / `WiParse-Deploy.zip` 目录式发布 |

## 2. 客户端模块

| 模块 | 路径 | 职责 |
|------|------|------|
| 清单与版本 | `crates/wiparse-core/src/update/` | 解析 `latest.json`、semver 比较、SHA256 校验 |
| 配置 | `config.json` → `update` 节 | 清单 URL、频道、检查间隔 |
| 下载与安装 | `crates/wiparse-gui/src/update/` | 后台下载、About 对话框 UI、调用安装脚本 |
| 安装脚本 | `packaging/update/apply-update.ps1` | 退出后解压 zip 到安装目录并重启 |
| 发布脚本 | `packaging/update/publish-release.ps1` | 本地打包后生成 manifest 并上传 |

### 配置示例 (`config.json`)

```json
{
  "update": {
    "enabled": true,
    "manifest_url": "https://update.example.com/wiparse/stable/latest.json",
    "channel": "stable",
    "check_interval_hours": 24,
    "auto_download": false
  }
}
```

- `manifest_url` 为空则禁用在线更新。
- `check_interval_hours`: 0 表示仅启动时检查一次；24 表示每 24 小时最多自动检查一次。
- `auto_download`: 预留；当前仍需用户在 About 中确认下载/安装。

### 更新清单格式 (`latest.json`)

参见 `packaging/update/latest.json.example`。

```json
{
  "product": "wiparse",
  "channel": "stable",
  "version": "1.0.2",
  "min_version": "1.0.0",
  "published_at": "2026-08-04T00:00:00Z",
  "notes": "修复波形显示；增强 Tek 采集",
  "notes_url": "https://docs.example.com/wiparse/1.0.2",
  "packages": [{
    "target": "windows-x64",
    "url": "https://update.example.com/wiparse/releases/1.0.2/WiParse-1.0.2-win64.zip",
    "size": 15728640,
    "sha256": "abc...64hex",
    "filename": "WiParse-1.0.2-win64.zip"
  }]
}
```

### 发布 zip 内容建议

与 `WiParse-Deploy/` 一致：

```
WiParse.exe
WiParse-CLI.exe
config.default.json
mcp/wiparse/...
```

**不要**在 zip 内附带用户 `config.json` 或业务数据目录。

## 3. 服务器部署要求

### 3.1 最低要求

| 项目 | 要求 |
|------|------|
| **协议** | HTTPS（TLS 1.2+），有效证书（Let's Encrypt / 企业 CA） |
| **托管方式** | 静态文件即可（Nginx、Caddy、IIS、阿里云 OSS、腾讯云 COS、AWS S3 + CloudFront） |
| **目录结构** | 见下方推荐布局 |
| **带宽** | 按用户量估算；单包约 15–40 MB，建议走 CDN |
| **CORS** | 不需要（原生客户端，非浏览器） |

### 3.2 推荐目录布局

```
https://update.example.com/
└── wiparse/
    └── stable/
        ├── latest.json              ← 客户端 manifest_url 指向此文件
        └── releases/
            ├── 1.0.1/
            │   └── WiParse-1.0.1-win64.zip
            └── 1.0.2/
                └── WiParse-1.0.2-win64.zip
```

可选 beta 频道：

```
/wiparse/beta/latest.json
```

客户端通过 `config.update.manifest_url` 选择频道。

### 3.3 Nginx 配置示例

```nginx
server {
    listen 443 ssl http2;
    server_name update.example.com;

    ssl_certificate     /etc/ssl/update.example.com/fullchain.pem;
    ssl_certificate_key /etc/ssl/update.example.com/privkey.pem;

    root /var/www/wiparse;
    index index.html;

    location / {
        autoindex off;
        add_header Cache-Control "public, max-age=300";
    }

    location = /wiparse/stable/latest.json {
        add_header Cache-Control "no-cache";
    }

    location ~* \.zip$ {
        add_header Cache-Control "public, max-age=31536000, immutable";
    }
}
```

### 3.4 对象存储（OSS/COS/S3）

1. 创建私有或公有读 Bucket（推荐公有读 + CDN）。
2. 上传 `releases/<version>/WiParse-x.y.z-win64.zip`。
3. 上传 `stable/latest.json`（**短缓存**或每次发布覆盖）。
4. CDN 回源绑定 Bucket；开启 HTTPS。
5. 将 CDN 域名写入客户端 `manifest_url` 与 manifest 内 `packages[].url`。

### 3.5 发布流程（CI/CD 建议）

1. `cargo build --release` 生成 exe。
2. 组装 `WiParse-Deploy.zip`（与现有手动流程一致）。
3. 运行 `packaging/update/publish-release.ps1` 计算 SHA256、生成 `latest.json`。
4. 上传 zip + manifest 到服务器（rsync / ossutil / aws s3 sync）。
5. （可选）对 zip 做 **Authenticode 代码签名**，减少 SmartScreen 拦截。

### 3.6 安全建议（生产环境）

| 级别 | 措施 |
|------|------|
| 必须 | HTTPS + SHA256 校验 |
| 推荐 | 固定 manifest 域名；内网可配 DNS/IP 白名单 |
| 进阶 | Ed25519  detached signature（manifest 或 zip）；客户端内置公钥 |
| 进阶 | Authenticode 签名 `WiParse.exe` |
| 运维 | 保留历史版本 zip；`min_version` 强制淘汰过旧客户端 |

## 4. 用户侧更新流程

1. 启动时（或 About → **检查更新**）拉取 `latest.json`。
2. 若 `version` > 当前版本 → 显示新版本号与 `notes`。
3. **下载更新** → 保存到 `%LOCALAPPDATA%\WiParse\updates\`，校验 SHA256。
4. **安装并重启** → 启动 `apply-update.ps1`，主程序退出后覆盖文件并重启。

## 5. 后续扩展（未实现）

- [ ] Ed25519 签名校验
- [ ] 增量更新（bsdiff / courgette）
- [ ] beta / stable 频道 UI 切换
- [ ] 独立 `WiParse-Updater.exe`（免 PowerShell 策略限制）
- [ ] CLI `wiparse update check|apply`

## 6. 快速验证

1. 本地启动 HTTP 文件服务器或上传 example manifest。
2. 在 `config.json` 设置 `update.manifest_url`。
3. 打开 **设置 → 关于 → 检查更新**。

本地测试可用 [miniserve](https://github.com/svenstaro/miniserve) 或 Python：

```powershell
cd packaging\update
python -m http.server 8443
# manifest_url 改为 https://... 需反向代理；内网测试可暂用自签证书
```

---

**当前版本号来源**：工作区 `Cargo.toml` → `1.0.1`。发布新版本前请 bump workspace `version` 并重新编译。
