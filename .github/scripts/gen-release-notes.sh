#!/usr/bin/env bash
# 生成 Release Notes：自上一语义化版本 tag 以来的提交，按 Conventional Commits 前缀分组。
# 首个 tag 无历史时退化为全量提交。输出 Markdown 到 stdout。
set -euo pipefail

TAG="${RELEASE_TAG:-${GITHUB_REF_NAME:-}}"

# 上一个 tag（按版本号排序取当前之前的第一个）；可能是空（首个发布）
PREV_TAG="$(git tag --sort=-v:refname | awk -v tag="$TAG" '$0 != tag && $0 ~ /^v[0-9]+\.[0-9]+\.[0-9]/ {print; exit}')"

if [ -n "$PREV_TAG" ]; then
  RANGE="$PREV_TAG..HEAD"
  HEADER_RANGE="$PREV_TAG → $TAG"
else
  RANGE="HEAD"
  HEADER_RANGE="首个发布 $TAG"
fi

echo "## 版本变化（$HEADER_RANGE）"
echo

# 无提交时兜底
if [ -z "$(git log --format=%s "$RANGE")" ]; then
  echo "本次版本无新的提交记录。"
  exit 0
fi

group() {
  local title="$1" pattern="$2"
  local lines
  lines="$(git log --no-merges --format='- %s (%h)' "$RANGE" | grep -E "$pattern" || true)"
  if [ -n "$lines" ]; then
    echo "### $title"
    echo
    echo "$lines"
    echo
  fi
}

group "✨ 新功能"    '^\s*- (feat|功能)'
group "🐛 修复"      '^\s*- (fix|修复)'
group "⚡ 性能优化"  '^\s*- perf'
group "♻️ 重构"      '^\s*- refactor'
group "📖 文档"      '^\s*- docs'
group "✅ 测试"      '^\s*- test'
group "🔧 其他（构建 / 杂项）" '^\s*- (chore|build|ci|style)'

# 不符合上述前缀的提交归入"其他变更"
OTHERS="$(git log --no-merges --format='- %s (%h)' "$RANGE" \
  | grep -vE '^\s*- (feat|功能|fix|修复|perf|refactor|docs|test|chore|build|ci|style)' || true)"
if [ -n "$OTHERS" ]; then
  echo "### 📦 其他变更"
  echo
  echo "$OTHERS"
  echo
fi

echo "---"
echo
FULL_SHA="$(git rev-parse HEAD)"
echo "完整提交历史：[\`$FULL_SHA\`](https://github.com/${GITHUB_REPOSITORY:-}/commits/$FULL_SHA)"
