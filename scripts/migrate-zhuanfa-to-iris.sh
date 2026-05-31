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

# db 改名（仅 master 节点有 db）— binary 在 [6/6] 处理（避免旧 zhuanfa binary 在新路径下被启动）
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
        # sed: 路径 / 二进制名 / db 文件名 / env 变量 / unit 描述都替换（含 Title Case）
        sed -E '
            s|/opt/zhuanfa|/opt/iris|g
            s|zhuanfa-master|iris-master|g
            s|zhuanfa-node|iris-node|g
            s|\bzhuanfa\.db|iris.db|g
            s|\bZF_|IRIS_|g
            s|\bzhuanfa\b|iris|g
            s|\bZhuanfa\b|Iris|g
            s|\bZHUANFA\b|IRIS|g
        ' "$src" > "$dst"
        echo "    wrote $dst (from $src)"
    elif [ ! -f "$dst" ]; then
        echo "    ⚠️  $src 不存在且 $dst 也不存在 — 你可能需要手动写 unit"
    fi
done

echo "==> [4b/6] 转换 EnvironmentFile (.env) 内容"
# systemd unit 经常用 EnvironmentFile=/opt/.../node.env 而不是 inline Environment=
# 这些 .env 文件不会被 [4/6] 处理。在这里做内容 sed。
# 注意：仅替换路径 / db 名 / env 变量名 / binary 名；不替换 Title Case Zhuanfa /
# 小写 zhuanfa 单词，避免破坏 .env 里含品牌字符串的值（如 admin 密码、token、注释等）
shopt -s nullglob
for f in "$NEW_DIR"/*.env "$NEW_DIR"/conf/*.env; do
    [ -f "$f" ] || continue
    sed -i -E '
        s|/opt/zhuanfa|/opt/iris|g
        s|\bzhuanfa\.db|iris.db|g
        s|\bZF_|IRIS_|g
        s|zhuanfa-master|iris-master|g
        s|zhuanfa-node|iris-node|g
    ' "$f"
    echo "    sed $f"
done
shopt -u nullglob

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

echo "==> [6/6] 环境就绪（不自动启动 — 等新 iris binary 推送完成后再 systemctl start）"
# 把旧 zhuanfa 二进制改名占位（仅为防止 iris-master.service ExecStart 路径报 Not Found；
# 真正可执行的 iris-* binary 由 deploy 脚本推送覆盖）
[ -f "$NEW_DIR/zhuanfa-master" ] && mv "$NEW_DIR/zhuanfa-master" "$NEW_DIR/iris-master.placeholder" 2>/dev/null
[ -f "$NEW_DIR/zhuanfa-node" ]   && mv "$NEW_DIR/zhuanfa-node"   "$NEW_DIR/iris-node.placeholder"   2>/dev/null
echo "    旧 binary 标记为 .placeholder（待新 iris binary 覆盖）"
echo
echo "迁移阶段完成。剩余操作（在 deploy 机器上）："
echo "    1) scp artifact/iris-master root@<host>:$NEW_DIR/iris-master"
echo "    2) scp artifact/iris-node   root@<host>:$NEW_DIR/iris-node"
echo "    3) chmod +x $NEW_DIR/iris-{master,node}"
echo "    4) systemctl start iris-master.service iris-node.service"
echo
echo "全部 OK 后清理："
for u in "${OLD_UNITS[@]}"; do
    [ -f "/etc/systemd/system/$u.service" ] && echo "    rm /etc/systemd/system/$u.service"
done
[ -f "$NEW_DIR/iris-master.placeholder" ] && echo "    rm $NEW_DIR/iris-master.placeholder"
[ -f "$NEW_DIR/iris-node.placeholder" ]   && echo "    rm $NEW_DIR/iris-node.placeholder"
