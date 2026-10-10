/**
 * 产品外部链接常量（T-016）。
 * TODO(产品)：官网 / 仓库地址待产品确定后替换（当前为 example.com 占位，AGENTS.md 约定不使用真实域名）。
 */
export const LINKS = {
  official: 'https://vibetype.example.com/',
  repository: 'https://github.com/vibetype/vibetype',
  releases: 'https://github.com/vibetype/vibetype/releases',
} as const

/** “跳过此版本”的本地存储键（about 配置节暂无该字段，v0.1 前端本地保存）。 */
export const SKIPPED_VERSION_KEY = 'vt.update.skipped_version'
