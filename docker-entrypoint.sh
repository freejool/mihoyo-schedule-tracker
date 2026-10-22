#!/bin/sh
# 容器入口：以 root 启动，修复 bind mount 数据目录属主后降权为 app 运行。
#
# 背景：Docker 在 Linux 上会以 root 预创建 bind mount 的宿主机源目录
# （如 ./data，属主 root:root 755）。镜像以非 root 的 app 用户运行，
# 对 root 属主目录无写权限 → SQLite 报 (code: 14) unable to open database file。
# 这里在每次启动时把 /app/data 属主交给 app（含既有 root 遗留文件），再降权执行。
# 降权用 setpriv（util-linux 自带，无需额外安装）。
set -e

mkdir -p /app/data
chown -R app:app /app/data

exec setpriv --reuid=app --regid=app --clear-groups "$@"
