# 部署模板

## master 系统服务

`iris-master.service` 是 master 的 systemd unit 模板。

### 安装步骤

```bash
# 1. 准备目录 + 配置
sudo mkdir -p /opt/iris/{data,certs}
sudo tee /opt/iris/master.env <<EOF
IRIS_JWT_SECRET=$(openssl rand -hex 32)
IRIS_ADMIN_USER=admin
IRIS_ADMIN_PASS=$(openssl rand -base64 24)
# 生产强烈建议：
# IRIS_REQUIRE_TLS=1
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

### 节点安装

master 启动后访问 `http://<MASTER>:7080`，登录 admin，创建节点 → 系统生成 enrollment token → 在节点服务器执行：

```bash
curl -fsSL http://<MASTER>:7080/install.sh | sudo bash -s -- \
  --master http://<MASTER>:7080 \
  --token <TOKEN>
```

详见根目录 README。
