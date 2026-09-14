# Testing Hub 插件市场：资料与阿里云部署

适用版本 **WiParse 1.1.11+**。服务端实现：`services/testing-hub-marketplace`（Node ≥18，零 npm 依赖）。  
本机演示仍用 [`services/testing-hub-marketplace/README.md`](../services/testing-hub-marketplace/README.md)。**上公网必须走 HTTPS**，见第 3 节。

配套模板：

| 文件 | 用途 |
|------|------|
| [`deploy/env.example`](../services/testing-hub-marketplace/deploy/env.example) | systemd 环境变量 |
| [`deploy/wiparse-marketplace.service`](../services/testing-hub-marketplace/deploy/wiparse-marketplace.service) | systemd 单元 |
| [`deploy/nginx-marketplace.conf`](../services/testing-hub-marketplace/deploy/nginx-marketplace.conf) | Nginx 反代 + TLS |

把文档里的 `market.example.com`、`CHANGE_ME_TOKEN` 换成你的域名和随机 token。

---

## 1. 这是什么

市场服务是 **插件目录 + zip 制品仓库**，不是 GUI，也不跑插件。工位上的 WiParse 只做浏览 / 下载 / 安装；插件真正执行仍在工位本机 Node + GUI HTTP API。

```
工位 WiParse.exe  (集成测试 → 市场)
        HTTPS GET /v1/catalog
        HTTPS GET /v1/plugins/:id/versions/:ver/download
                │
                ▼
阿里云 ECS / 轻量应用服务器
  安全组只开 22 / 80 / 443（不要开 8787）
  Nginx :443  TLS（阿里云免费 SSL 或 certbot）
        proxy_pass http://127.0.0.1:8787
  Node  127.0.0.1:8787
        data/index.json
        data/artifacts/<id>/<version>.zip
```

发布走另一条路（CI 或管理员机）：`POST /v1/plugins/:id/versions`，Bearer token。目录与下载 **无鉴权**——知道 URL 的人就能拉 zip。工位插件若含工艺/密钥，把市场放在专有网络、VPN 或 IP 白名单后。

---

## 2. 仓库对照（上云只带这些）

| 路径 | 上云？ | 说明 |
|------|--------|------|
| `services/testing-hub-marketplace/src/` | 要 | `index.mjs` / `server.mjs` / `store.mjs` |
| `services/testing-hub-marketplace/package.json` | 要 | 无 dependencies |
| `services/testing-hub-marketplace/deploy/` | 要 | systemd + nginx |
| `scripts/seed-marketplace-plugins.mjs` | 可选 | 只在你要演示插件时 |
| `test-tools/` 整棵 | 不要上服务器 | 工位客户端 |
| `fixtures/`、演示 zip | 不要上生产 | `dev-token` 与样例插件 |

服务启动：

```bash
export MARKETPLACE_PUBLISH_TOKENS='<长随机串>'
export MARKETPLACE_DATA_DIR=/var/lib/wiparse-marketplace
export HOST=127.0.0.1
export PORT=8787
node src/index.mjs --host 127.0.0.1 --port 8787 --data "$MARKETPLACE_DATA_DIR"
```

默认 `HOST=127.0.0.1`。公网 ECS **不要** `HOST=0.0.0.0` 直出 8787。

环境变量：

| 变量 | 默认 | 含义 |
|------|------|------|
| `PORT` | `8787` | Node 监听端口 |
| `HOST` | `127.0.0.1` | 绑定地址 |
| `MARKETPLACE_DATA_DIR` | `./data` | 索引 + zip |
| `MARKETPLACE_PUBLISH_TOKENS` | 空 | 逗号分隔 Bearer；**空则发布/删除全部 401** |

---

## 3. 客户端硬约束（上云必读）

GUI（`wiparse_core::marketplace`）和 Node（`test-tools/lib/marketplace-client.mjs`）同一规则：

| URL | 工位能否连 |
|-----|------------|
| `https://market.example.com` | 可以（生产唯一推荐） |
| `http://127.0.0.1:8787` / `localhost` / `::1` | 可以（本机演示） |
| `http://<公网IP>:8787` | **拒绝**，错误 `URL must use HTTPS` |
| 其它明文 HTTP | 仅当工位置 `WIPARSE_MARKETPLACE_ALLOW_HTTP=1`；**不要**在产线开 |

工位 `config.json`：

```json
"apps": {
  "test_tool": {
    "marketplace": {
      "enabled": true,
      "base_url": "https://market.example.com",
      "install_dir": "",
      "channel": "stable"
    }
  }
}
```

- `install_dir` 空 → `{data_root}/marketplace`
- 环境变量 `WIPARSE_MARKETPLACE_URL` 覆盖 `base_url`
- 频道：`stable` / `beta` / `internal`（目录按 **最新版本** 的 `channel` 过滤）

---

## 4. HTTP API（服务版本 0.1.0）

基址后面不要多余斜杠。JSON 响应信封：`{ "ok": true, ... }` 或 `{ "ok": false, "error": { "code", "message" } }`。

| 方法 | 路径 | 鉴权 | 作用 |
|------|------|------|------|
| GET | `/` 或 `/v1` | 无 | 服务名 + 路由列表 |
| GET | `/v1/health` | 无 | `{ ok, service, version }` |
| GET | `/v1/catalog` | 无 | 目录。查询：`channel` `type` `q` |
| GET | `/v1/plugins/:id` | 无 | 全部版本（semver 新→旧） |
| GET | `/v1/plugins/:id/versions/:version` | 无 | 单版本 meta（含 sha256 / size / download_path） |
| GET | `/v1/plugins/:id/versions/:version/download` | 无 | zip，`Content-Type: application/zip` |
| POST | `/v1/plugins/:id/versions` | `Authorization: Bearer <token>` | 发布 |
| DELETE | `/v1/plugins/:id/versions/:version` | Bearer | 下架；最后一个版本会删掉该插件目录 |

发布 body：

```json
{
  "meta": {
    "id": "my-plugin",
    "version": "1.0.0",
    "name": "My Plugin",
    "name_zh": "我的插件",
    "type": "custom",
    "publisher": "wiparse",
    "channel": "stable",
    "sha256": "<64 hex of zip>",
    "size": 12345
  },
  "artifact_base64": "<zip bytes as base64>"
}
```

`meta.id` 必须等于路径 `:id`。服务端重算 zip 的 SHA-256 / 字节数，与 meta 不一致返回 `E_HASH`（HTTP 400）。

错误码：`E_AUTH` `E_NOT_FOUND` `E_LAYOUT` `E_HASH` `E_INTERNAL`。

### 磁盘布局（服务器）

```
$MARKETPLACE_DATA_DIR/
  index.json
  artifacts/
    <id>/
      <version>.zip
```

`index.json` 结构：`{ version: 1, updated_at, plugins: { <id>: { id, versions: { <ver>: meta } } } }`。  
写入用临时文件 + rename，单进程安全；不要多实例写同一目录。

### 工位安装后布局

```
{data_root}/marketplace/
  registry.json
  plugins/<id>/<version>/plugin.json
  plugins/<id>/<version>/index.mjs
```

Runner 合并捆绑 `test-tools/plugins/` 与市场 **active** 版本；同 id 时市场覆盖捆绑。

### zip 包约定

- 根目录或单层子目录里必须有 `plugin.json`（id / version 与 meta 一致）
- 入口默认 `index.mjs`，生命周期见 [`test-tools/PLUGIN_SPEC.md`](../test-tools/PLUGIN_SPEC.md)
- 客户端解压：仅 store / deflate，上限约 5000 文件 / 200 MB 解压后
- `sandbox.permissions` **只声明不强制**（`gui.api` `cli` `serial` `fs.data_root` `fs.plugin_dir` `network.outbound`）
- 可选 `signature`：Ed25519，对 **sha256 的 hex 字符串**（UTF-8）签名；服务端存储、客户端在配置公钥时校验

meta schema：[`test-tools/schemas/marketplace-package.schema.json`](../test-tools/schemas/marketplace-package.schema.json)。

---

## 5. 阿里云部署方案

推荐：**轻量应用服务器**（2 核 2G、40 GB 盘、Ubuntu 22.04 或 Alibaba Cloud Linux 3）+ **域名** + **免费 SSL**。流量很小（JSON + 小 zip），不必 RDS / OSS / K8s。制品若会涨到数 GB，再把 `data/artifacts` 挂到独立云盘。

### 5.1 控制台

1. 购买实例，绑弹性公网 IP（轻量通常自带）。
2. 安全组 / 防火墙：**入方向只放 22、80、443**。不要放 8787。
3. 域名解析 A 记录到该公网 IP（如 `market.yourcompany.com`）。
4. （可选）申请阿里云免费 SSL，或用 certbot 自动签 Let's Encrypt。

### 5.2 服务器安装

```bash
# Ubuntu 22.04 示例
sudo apt-get update
sudo apt-get install -y nginx curl unzip
# Node 20 LTS
curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
sudo apt-get install -y nodejs
node -v   # >= 18

sudo useradd --system --home /opt/wiparse-marketplace --shell /usr/sbin/nologin wiparse
sudo mkdir -p /opt/wiparse-marketplace /var/lib/wiparse-marketplace /etc/wiparse-marketplace
sudo chown -R wiparse:wiparse /opt/wiparse-marketplace /var/lib/wiparse-marketplace
```

从开发机同步 **仅服务端源码**（不要带 `data/server.pid`、本地演示 zip）：

```powershell
# 开发机（仓库根）
scp -r services/testing-hub-marketplace/src `
        services/testing-hub-marketplace/package.json `
        services/testing-hub-marketplace/deploy `
    root@YOUR_EIP:/opt/wiparse-marketplace/
```

Linux 开发机：

```bash
rsync -av --exclude data --exclude fixtures --exclude test \
  services/testing-hub-marketplace/ root@YOUR_EIP:/opt/wiparse-marketplace/
```

### 5.3 密钥与 systemd

```bash
# 生成发布 token（不要用 dev-token）
TOKEN=$(openssl rand -hex 32)
echo "MARKETPLACE_PUBLISH_TOKENS=$TOKEN" | sudo tee /etc/wiparse-marketplace/env
sudo tee -a /etc/wiparse-marketplace/env >/dev/null <<'EOF'
HOST=127.0.0.1
PORT=8787
MARKETPLACE_DATA_DIR=/var/lib/wiparse-marketplace
EOF
sudo chmod 600 /etc/wiparse-marketplace/env
sudo chown root:root /etc/wiparse-marketplace/env

sudo cp /opt/wiparse-marketplace/deploy/wiparse-marketplace.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now wiparse-marketplace
sudo systemctl status wiparse-marketplace --no-pager
curl -fsS http://127.0.0.1:8787/v1/health
```

把 `$TOKEN` 存到密码管理器。丢了只能改 env 再 `systemctl restart`。

### 5.4 Nginx + TLS

把 `deploy/nginx-marketplace.conf` 拷到 `/etc/nginx/sites-available/wiparse-marketplace`，改 `server_name` 和证书路径。

**certbot（Let's Encrypt）：**

```bash
sudo apt-get install -y certbot python3-certbot-nginx
# 先用 conf 里 HTTP server 块，再：
sudo certbot --nginx -d market.example.com
sudo nginx -t && sudo systemctl reload nginx
```

**阿里云下载的 PEM：** 放到 `/etc/nginx/ssl/`，在 conf 里填 `ssl_certificate` / `ssl_certificate_key`。

关键点：

- `client_max_body_size 64m;` —— 发布是 **zip 的 base64 JSON**，体积约 zip × 1.37，64 MB 大约对应 ~45 MB zip
- 反代到 `http://127.0.0.1:8787`
- 不要在 Nginx 再开 CORS 需求（工位是原生 HTTPS 客户端，不是浏览器）

验证：

```bash
curl -fsS https://market.example.com/v1/health
curl -fsS https://market.example.com/v1/catalog
```

证书必须被 **Windows 工位信任**。自签证书会导致 GUI 拉目录失败。

### 5.5 不要做的事

- 安全组放行 8787，或 `HOST=0.0.0.0` 直接对公网
- 工位填 `http://x.x.x.x:8787`（会被客户端拒绝）
- 生产使用 `dev-token` 或播种演示插件当正式目录
- 多进程 / 多机共享同一 `data/`（无分布式锁）
- 把市场 URL 写进公开文档却不配 IP 白名单（目录无登录）

---

## 6. 发布插件（管理员机）

zip 内至少：

```
plugin.json
index.mjs
```

`plugin.json.id` / `version` 必须与 meta 一致。可用仓库里的打包函数，或任意 zip 后自己算 sha256。

```bash
# 开发机，仓库根。Node 18+
cd /path/to/WiParse-R

# 把插件目录打成 zip（也可用 7-Zip；客户端不支持 zip64 / 加密）
# 下面用一段内联脚本生成 meta + 发布
node --input-type=module <<'EOF'
import fs from "node:fs";
import { zipDirectory, buildMetaFromZip } from "./test-tools/lib/marketplace-install.mjs";
import { createMarketplaceClient } from "./test-tools/lib/marketplace-client.mjs";

const dir = process.env.PLUGIN_DIR;
const url = process.env.MARKET_URL;       // https://market.example.com
const token = process.env.MARKET_TOKEN;
const zip = zipDirectory(dir);
const manifest = JSON.parse(fs.readFileSync(dir + "/plugin.json", "utf8"));
const meta = buildMetaFromZip(zip, {
  id: manifest.id,
  version: manifest.version,
  name: manifest.name,
  name_zh: manifest.name_zh,
  type: manifest.type,
  description: manifest.description,
  publisher: manifest.publisher || "wiparse",
  channel: manifest.marketplace?.channel || "stable",
  engines: manifest.engines,
  sandbox: manifest.sandbox,
});
const client = createMarketplaceClient({ baseUrl: url, token });
const r = await client.publish(manifest.id, {
  meta,
  artifactBase64: zip.toString("base64"),
});
console.log(JSON.stringify(r, null, 2));
EOF
```

PowerShell：

```powershell
$env:PLUGIN_DIR = "D:\plugins\my-plugin"
$env:MARKET_URL = "https://market.example.com"
$env:MARKET_TOKEN = "<token>"
# 同上 node 脚本
```

下架：

```bash
curl -X DELETE \
  -H "Authorization: Bearer $TOKEN" \
  https://market.example.com/v1/plugins/my-plugin/versions/1.0.0
```

仅本机验证（loopback HTTP）：

```powershell
.\scripts\deploy-marketplace.ps1
node test-tools\marketplace.mjs catalog --url http://127.0.0.1:8787 --json
```

---

## 7. 工位验收

1. 工位 `config.json` 的 `apps.test_tool.marketplace.base_url` = `https://market.example.com`，`enabled` = true。
2. 打开 WiParse → **集成测试** → **市场** → 展开服务器确认 URL → 刷新。
3. 应看到已发布插件；点安装。
4. 切回 **插件**，列表出现该 id，`source=marketplace`。
5. 预检 / 运行走 `runner.mjs`，与捆绑插件相同生命周期。

CLI 等价：

```powershell
cd test-tools
node marketplace.mjs catalog --url https://market.example.com --json
node marketplace.mjs pull --plugin my-plugin --version 1.0.0 `
  --url https://market.example.com --data-root <工位数据根>
node runner.mjs --plugin my-plugin --lifecycle preflight --data-root <工位数据根>
```

---

## 8. 备份与运维

```bash
# 备份（停写或接受短暂不一致）
sudo tar -C /var/lib -czf /root/wiparse-marketplace-$(date +%F).tgz wiparse-marketplace

# 日志
sudo journalctl -u wiparse-marketplace -f

# 升级服务端：覆盖 /opt/wiparse-marketplace/src 后
sudo systemctl restart wiparse-marketplace
```

`index.json` 与 `artifacts/` 必须一起备份。只拷 zip 没有索引则目录为空。

健康检查可挂阿里云拨测：`GET https://market.example.com/v1/health` 期望 200 且 `ok: true`。

---

## 9. 现状边界（1.1.11 Phase 1）

上云前按真实风险接受这些限制：

| 项 | 现状 |
|----|------|
| 目录/下载鉴权 | 无；URL 即权限 |
| TLS | Node 只听明文；必须 Nginx/Caddy |
| 发布体积 | 整包 base64 进内存，无分块 |
| 多实例 | 不支持同一 data 目录 |
| 沙箱 | 清单字段，运行时不隔离 |
| 签名 | 可选；服务端不验，客户端可验 |
| 账号 / 审核流 | 无 |
| CORS | 响应带 `Access-Control-Allow-Origin: *`（工位不依赖） |

内网工位、插件不敏感：ECS + HTTPS + 强 token 即可。对外公开或含工艺机密：加 Nginx `allow` IP、VPN，或前面再加鉴权网关（需改客户端，当前 GET 不带 token）。
