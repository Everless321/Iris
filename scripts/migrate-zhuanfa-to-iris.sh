#!/usr/bin/env bash
# zhuanfa → iris 一次性 prod 节点迁移脚本。
# 在已有 zhuanfa 部署的远端节点上跑，原地切到 iris：
#   1) stop & disable zhuanfa-{master,node}.service
#   2) /opt/zhuanfa → /opt/iris，二进制改名，db 改名
#   3) 删 server cert + 版本标记 → 让 iris-master 启动时自动重签（SAN=iris-master）
#      node cert / CA 保留（SAN 含 node_id 不变，CA 信任根不动）
#   4) 从旧 systemd unit 抽 env 转换 ZF_→IRIS_ 写新 unit
#   5) systemctl daemon-reload + enable iris-{master,node}.service
#   6) 启动新服务并验证
#
# 用法：本地执行 → ssh 推到每个节点；或者直接在节点上 root 执行。
# 推荐先 cp 整个 /opt/zhuanfa 备份再跑：cp -a /opt/zhuanfa /opt/zhuanfa.bak.$(date +%s)
set -euo pipefail

OLD_DIR=/opt/zhuanfa
NEW_DIR=/opt/iris
OLD_DB=zhuanfa.db
NEW_DB=iris.db
OLD_UNITS=(zhuanfa-master zhuanfa-node)
NEW_UNITS=(iris-master iris-node)

[ "$(id -u)" -eq 0 ] || { echo "需要 root"; exit 1; }

echo "==> [1/6] 停止旧服务"
for u in "${OLD_UNITS[@]}"; do
    if systemctl is-active "$u.service" >/dev/null 2>&1; then
        echo "    systemctl stop $u.service"
        systemctl stop "$u.service" || true
    fi
done

echo "==> [2/6] 迁移 $OLD_DIR → $NEW_DIR"
if [ -d "$OLD_DIR" ] && [ ! -e "$NEW_DIR" ]; then
    mv "$OLD_DIR" "$NEW_DIR"
    echo "    moved"
elif [ -d "$NEW_DIR" ]; then
    echo "    $NEW_DIR 已存在，跳过 mv"
else
    echo "    $OLD_DIR 不存在，跳过"
fi

# 二进制 + db 改名（按存在性判断；master 节点有 db，node 节点没有）
[ -f "$NEW_DIR/zhuanfa-master" ] && mv "$NEW_DIR/zhuanfa-master" "$NEW_DIR/iris-master" && echo "    master binary renamed"
[ -f "$NEW_DIR/zhuanfa-node" ]   && mv "$NEW_DIR/zhuanfa-node"   "$NEW_DIR/iris-node"   && echo "    node binary renamed"
if [ -f "$NEW_DIR/data/$OLD_DB" ]; then
    mv "$NEW_DIR/data/$OLD_DB" "$NEW_DIR/data/$NEW_DB"
    echo "    db renamed → $NEW_DB"
    # sqlite WAL/SHM 副文件也要跟着移（如果存在）
    [ -f "$NEW_DIR/data/$OLD_DB-wal" ] && mv "$NEW_DIR/data/$OLD_DB-wal" "$NEW_DIR/data/$NEW_DB-wal"
    [ -f "$NEW_DIR/data/$OLD_DB-shm" ] && mv "$NEW_DIR/data/$OLD_DB-shm" "$NEW_DIR/data/$NEW_DB-shm"
fi

echo "==> [3/6] 清 server cert + 版本标记（让 iris-master 自动重签 SAN=iris-master）"
# 仅 master 节点会执行 ensure_dev_certs；node 节点没这堆 cert 也无所谓。
# CA + 已签发的 node client cert 保留（信任根不变 + SAN 含 node_id 不变）
rm -f "$NEW_DIR/certs/server.pem" "$NEW_DIR/certs/server-key.pem" "$NEW_DIR/certs/.server-cert-version"
echo "    cleared server cert (if existed)"

echo "==> [4/6] 转换 systemd unit 文件"
for i in 0 1; do
    old_u="${OLD_UNITS[$i]}"
    new_u="${NEW_UNITS[$i]}"
    src="/etc/systemd/system/$old_u.service"
    dst="/etc/systemd/system/$new_u.service"
    if [ -f "$src" ]; then
        # sed: 路径 / 二进制名 / db 文件名 / env 变量 / unit 描述都替换
        sed -E '
            s|/opt/zhuanfa|/opt/iris|g
            s|zhuanfa-master|iris-master|g
            s|zhuanfa-node|iris-node|g
            s|\bzhuanfa\.db|iris.db|g
            s|\bZF_|IRIS_|g
            s|\bzhuanfa\b|iris|g
        ' "$src" > "$dst"
        echo "    wrote $dst (from $src)"
    elif [ ! -f "$dst" ]; then
        echo "    ⚠️  $src 不存在且 $dst 也不存在 — 你可能需要手动写 unit"
    fi
done

systemctl daemon-reload

echo "==> [5/6] disable 旧 unit + enable 新 unit"
for u in "${OLD_UNITS[@]}"; do
    if systemctl is-enabled "$u.service" >/dev/null 2>&1; then
        systemctl disable "$u.service" 2>/dev/null || true
        echo "    disabled $u.service"
    fi
done
for i in 0 1; do
    new_u="${NEW_UNITS[$i]}"
    if [ -f "/etc/systemd/system/$new_u.service" ]; then
        systemctl enable "$new_u.service" 2>/dev/null && echo "    enabled $new_u.service"
    fi
done

echo "==> [6/6] 启动新服务并验证"
started=()
for i in 0 1; do
    new_u="${NEW_UNITS[$i]}"
    if [ -f "/etc/systemd/system/$new_u.service" ]; then
        systemctl start "$new_u.service" && started+=("$new_u") && echo "    started $new_u.service"
    fi
done

sleep 3
for u in "${started[@]}"; do
    if systemctl is-active "$u.service" >/dev/null 2>&1; then
        echo "    ✅ $u.service active"
    else
        echo "    ❌ $u.service NOT active — journalctl -u $u --since '1 min ago'"
    fi
done

# master 节点：调 /healthz
if systemctl is-active iris-master.service >/dev/null 2>&1; then
    if curl -fsS http://127.0.0.1:7080/healthz >/dev/null; then
        echo "    ✅ iris-master /healthz OK"
    else
        echo "    ⚠️  iris-master /healthz 失败"
    fi
fi

echo
echo "迁移完成。如果一切 OK 可以清理旧 unit 文件："
for u in "${OLD_UNITS[@]}"; do
    [ -f "/etc/systemd/system/$u.service" ] && echo "    rm /etc/systemd/system/$u.service"
done
echo
echo "本地（运维机器）操作提示："
echo "    mv ~/.zhuanfa ~/.iris  # 配置目录"
