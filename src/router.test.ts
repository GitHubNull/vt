import { describe, it, expect } from 'vitest'
import { NAV_ORDER, buildNavModules, collectRoutePaths, routes } from './router'

describe('路由表与导航元信息一致性（T-002）', () => {
  it('侧边栏模块与 PRD 第 2 章导航体系一致（编程首位，无“音视频转录”）', () => {
    const modules = buildNavModules()
    expect(modules.map((m) => m.key)).toEqual([...NAV_ORDER])
    expect(modules.map((m) => m.key)).toEqual([
      'coding',
      'dict',
      'ai',
      'general',
      'me',
      'stats',
      'faq',
      'about',
    ])
    expect(modules.map((m) => m.key)).not.toContain('transcription')
  })

  it('侧边栏无重复渲染：模块 key 唯一，各模块分组路径唯一', () => {
    const modules = buildNavModules()
    // 侧边栏每个模块只渲染一次
    expect(new Set(modules.map((m) => m.key)).size).toBe(modules.length)
    // 模块跳转路径指向其首个分组（有意同值），分组路径在模块内唯一
    for (const m of modules) {
      expect(m.path).toBe(m.children[0]?.path ?? m.path)
      const childPaths = m.children.map((c) => c.path)
      expect(new Set(childPaths).size).toBe(childPaths.length)
    }
  })

  it('通用模块分组与 T-002 规定路由一一对应', () => {
    const general = buildNavModules().find((m) => m.key === 'general')
    expect(general).toBeDefined()
    expect(general!.children.map((c) => c.path)).toEqual([
      '/general/interface',
      '/general/input',
      '/general/output',
      '/general/recording',
      '/general/system',
      '/general/models',
    ])
  })

  it('每个导航项路径在路由表中都有对应记录', () => {
    const routePaths = new Set(collectRoutePaths(routes))
    const navPaths = buildNavModules().flatMap((m) => [
      m.path,
      ...m.children.map((c) => c.path),
    ])
    for (const path of navPaths) {
      expect(routePaths.has(path)).toBe(true)
    }
  })

  it('导航元信息快照', () => {
    expect(buildNavModules()).toMatchSnapshot()
  })
})
