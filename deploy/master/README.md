# Master 部署模板

## 推荐：一键安装

自动生成 admin 密码 + JWT 密钥 + systemd unit + 健康检查 + 打印登录凭据：

```bash
curl -fsSL https://raw.githubusercontent.com/Everless321/Iris/main/install.sh | sudo bash -s -- --install-master
```

可选参数（不传则随机生成并明文打印一次）：

```bash
curl -fsSL https://raw.githubusercontent.com/Everless321/Iris/main/install.sh | sudo bash -s -- --install-master \
  --admin-user admin \
  --admin-pass <your-pass> \
  --jwt-secret $(openssl rand -hex 32)
```

升级现有 master 用 `--upgrade-master`（同一脚本，备份 + 替换 + 重启 + 失败自动回滚）：

```bash
curl -fsSL https://raw.githubusercontent.com/Everless321/Iris/main/install.sh | sudo bash -s -- --upgrade-master
```

## 手工安装（备用 / 高级）

不想 `curl | sudo bash`，或要做定制（如 `IRIS_REQUIRE_TLS=1` / `IRIS_SESSION_RETAIN_DAYS`）：

```bash
# 1. 准备目录 + 配置
sudo mkdir -p /opt/iris/{data,certs}
sudo tee /opt/iris/master.env <<EOF
IRIS_JWT_SECRET=$(openssl rand -hex 32)
IRIS_ADMIN_USER=admin
IRIS_ADMIN_PASS=$(openssl rand -base64 24)
# 生产强烈建议:
# IRIS_REQUIRE_TLS=1
# 会话明细保留: 0=永久全量 (默认), N=N天后聚合到 hourly 表 + DELETE 明细
# IRIS_SESSION_RETAIN_DAYS=0
# IRIS_SESSION_HOURLY_RETAIN_DAYS=0
EOF
sudo chmod 600 /opt/iris/master.env

# 2. 下载 master 二进制
sudo curl -fsSL -o /opt/iris/iris-master \
  https://github.com/Everless321/Iris/releases/latest/download/iris-master-musl-x86_64
sudo chmod 755 /opt/iris/iris-master

# 3. 安装 systemd unit
sudo cp iris-master.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now iris-master

# 4. 看启动日志
sudo journalctl -u iris-master -f
```

## 节点安装

master 启动后访问 `http://<MASTER>:7080`，登录 admin，创建节点 → 系统生成 enrollment token → 节点服务器执行:

```bash
curl -fsSL https://raw.githubusercontent.com/Everless321/Iris/main/install.sh | sudo bash -s -- \
  --master http://<MASTER>:7080 \
  --token <TOKEN>
```

详见根目录 README。
