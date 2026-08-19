#!/usr/bin/env bash
# sync-upstream.sh — 同步上游 tag 到 main，并自动恢复 fork 定制文件（README 等）
#
# 用法：./scripts/sync-upstream.sh <upstream-tag>
#   例：./scripts/sync-upstream.sh v3.20.0
#
# 前置：已配置 upstream remote
#   git remote add upstream https://github.com/farion1231/cc-switch.git
#   git config remote.upstream.tagOpt --no-tags
#
# 作用：
#   1. git fetch upstream
#   2. main 分支 reset --hard 到上游 tag（此时 README 会被重置为上游版本）
#   3. 从 local/main 恢复 fork 定制 README.md，单独 commit
#   4. force-with-lease push main
#   5. local/main rebaseline 到新 main（README 冲突保持 local/main 版本）
#
# 注意：reset --hard 不可逆，脚本会在脏工作区中止。

set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "用法: $0 <upstream-tag>" >&2
  echo "  例: $0 v3.20.0" >&2
  exit 1
fi

UPSTREAM_TAG="$1"

# 校验 upstream remote 已配置
if ! git remote get-url upstream >/dev/null 2>&1; then
  echo "❌ 未配置 upstream remote，先执行：" >&2
  echo "   git remote add upstream https://github.com/farion1231/cc-switch.git" >&2
  echo "   git config remote.upstream.tagOpt --no-tags" >&2
  exit 1
fi

# 校验本地分支存在
for branch in main local/main; do
  if ! git rev-parse --verify "$branch" >/dev/null 2>&1; then
    echo "❌ 缺少分支: $branch" >&2
    exit 1
  fi
done

# 工作区必须干净（reset --hard 会丢失未提交改动）
if [ -n "$(git status --porcelain)" ]; then
  echo "❌ 工作区不干净，请先提交或 stash：" >&2
  git status --short >&2
  exit 1
fi

# 签入身份
git config user.name "lzm04521"
git config user.email "lzm04521@126.com"

echo "==> git fetch upstream"
git fetch upstream

# remote.upstream.tagOpt=--no-tags 时裸 fetch 不拉 tag；upstream/<tag> 也不是合法 ref，
# 优先用本地 tag（需先 git fetch upstream --tags），其次尝试 upstream/<tag>（配置了 tag refspec 的仓库）
RESET_TARGET=""
if git rev-parse --verify -q "refs/tags/$UPSTREAM_TAG" >/dev/null; then
  RESET_TARGET="refs/tags/$UPSTREAM_TAG"
elif git rev-parse --verify -q "upstream/$UPSTREAM_TAG" >/dev/null; then
  RESET_TARGET="upstream/$UPSTREAM_TAG"
else
  echo "❌ 未找到 tag: $UPSTREAM_TAG（先执行 git fetch upstream --tags）" >&2
  exit 1
fi

echo "==> main reset --hard $RESET_TARGET"
git checkout main
git reset --hard "$RESET_TARGET"

echo "==> 从 local/main 恢复 fork 定制 README.md"
git checkout local/main -- README.md
if git diff --cached --quiet; then
  echo "   README 与上游一致，无需恢复 commit"
else
  git commit -m "docs: restore fork README after upstream sync

上游同步 (reset --hard upstream/$UPSTREAM_TAG) 后，README 被重置为上游版本。
此 commit 从 local/main 恢复 fork 定制 README。" --signoff
fi

echo "==> push main（force-with-lease）"
git push origin main --force-with-lease

echo "==> local/main rebaseline 到新 main"
git checkout local/main
if git rebase main; then
  :
else
  echo "⚠️  rebase 冲突，请手动解决。" >&2
  echo "   README.md 冲突时保持 local/main 版本：git checkout --ours README.md && git add README.md" >&2
  echo "   解决后：git rebase --continue" >&2
  exit 1
fi

echo "==> push local/main（force-with-lease）"
git push origin local/main --force-with-lease

git checkout local/v3.19.2-1 2>/dev/null || git checkout main

cat <<'EOF'

✅ 上游同步完成。接下来切发版分支：
   git checkout -b local/v<上游版本>-exp.<N>
   同步改三处 version（src-tauri/tauri.conf.json / package.json / src-tauri/Cargo.toml）
   git tag v<上游版本>-exp.<N>
   git push origin local/v<上游版本>-exp.<N> && git push origin v<上游版本>-exp.<N>
EOF
