//! 后处理管线框架（T-011，tech/05 5.1、5.3.4）：有序 Stage 列表 + PipelineContext。
//!
//! - `Stage` trait：每个后处理步骤一个实现（v0.1 替换词典；v0.5 预留口令 / AI Stage
//!   注册进 `PostProcess::extra`）；
//! - 有序执行：按注册顺序执行启用的 Stage，任一 Stage 可短路（`PipelineContext.short_circuit`，
//!   v0.5 AI Stage 需要：改写命中后跳过后续 Stage）；
//! - 独立开关：Stage 开关绑定配置（替换词典开关 = 配置 `dict.enabled`，产品原则 3：
//!   自动改写可关闭——关闭时识别原文直出）；
//! - 规则 / 开关缓存：Stage 的 `process` 为同步方法，规则快照与开关在 `refresh` 中
//!   异步刷新（`dict:changed` / `config:changed(dict)` 事件驱动）；
//! - 管线接入：消费 AsrService 的 `Recognized` → 管线 → 广播 `PipelineOutput`
//!   （T-012 由 PipelineService 消费：上屏 + 统一结果反馈 + 耗时埋点）。
//!
//! 隐私（NFR-S-02）：管线只经事件通道传递文本，绝不写日志。

use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;
use tracing::info;

use crate::events::AppEvent;
use crate::services::config_store::{ConfigSection, ConfigStore};
use crate::services::dict::{DictService, ReplaceRule};
use crate::services::session::SessionTracker;
use crate::services::stages::{
    FilterFillersStage, RemoveTrailingPunctStage, SmartNumberStage, TraditionalChineseStage,
};

/// 管线上下文：Stage 间传递的文本与状态。
#[derive(Debug, Clone)]
pub struct PipelineContext {
    /// 录音会话 ID（贯穿音频 → 识别 → 上屏链路）。
    pub session_id: u64,
    /// 当前文本（Stage 就地改写）。
    pub text: String,
    /// 短路标记：置位后跳过后续 Stage（v0.5 AI Stage 预留）。
    pub short_circuit: bool,
}

impl PipelineContext {
    pub fn new(session_id: u64, text: String) -> Self {
        Self {
            session_id,
            text,
            short_circuit: false,
        }
    }
}

/// 后处理 Stage 抽象（tech/05 5.3.4）。
pub trait Stage: Send + Sync {
    /// Stage 名（日志与测试标识）。
    fn name(&self) -> &'static str;

    /// 开关状态：关闭时该 Stage 跳过（产品原则 3：自动改写可关闭）。
    fn enabled(&self) -> bool;

    /// 处理文本；可设置 `ctx.short_circuit = true` 短路后续 Stage。
    fn process(&self, ctx: &mut PipelineContext);
}

/// 替换词典 Stage 的缓存快照（规则 + 总开关；`refresh` 异步刷新）。
#[derive(Debug, Clone, Default)]
struct ReplaceCache {
    enabled: bool,
    rules: Vec<ReplaceRule>,
}

/// 替换词典 Stage（F-DICT-01、PRD 3.4.1 业务规则 2）。
pub struct ReplaceStage {
    dict: DictService,
    config: ConfigStore,
    cache: Mutex<ReplaceCache>,
}

impl ReplaceStage {
    pub fn new(dict: DictService, config: ConfigStore) -> Self {
        Self {
            dict,
            config,
            cache: Mutex::new(ReplaceCache::default()),
        }
    }

    /// 异步刷新规则快照与开关缓存（`dict:changed` / `config:changed(dict)` 时调用）。
    pub async fn refresh(&self) {
        let enabled = self.config.get().await.dict.enabled;
        let rules = self.dict.enabled_snapshot().await;
        if let Ok(mut cache) = self.cache.lock() {
            cache.enabled = enabled;
            cache.rules = rules;
        }
    }
}

impl Stage for ReplaceStage {
    fn name(&self) -> &'static str {
        "replace_dict"
    }

    fn enabled(&self) -> bool {
        self.cache
            .lock()
            .map(|cache| cache.enabled)
            .unwrap_or(false)
    }

    fn process(&self, ctx: &mut PipelineContext) {
        let cache = match self.cache.lock() {
            Ok(cache) => cache.clone(),
            Err(_) => return,
        };
        let before = ctx.text.clone();
        ctx.text = apply_replace_rules(&ctx.text, &cache.rules);
        if ctx.text != before {
            info!(stage = %self.name(), "替换词典已修正识别文本");
        }
    }
}

/// 纯函数替换引擎（PRD 3.4 规则；单测逐条覆盖验收用例）。
///
/// - `rules` 须已按错误词长度降序排列（长词优先，见 `DictService::enabled_snapshot`）；
/// - 完整匹配错误词：英文错误词不区分大小写（`eq_ignore_ascii_case`，仅影响 ASCII）；
/// - 拼音口令空格修正：错误词为纯 ASCII 字母 / 数字时，文本中拉丁字母之间的
///   空格不影响匹配（ASR 常把连续拉丁字母识别为 "h a o f a n" 或 "hao fan"，
///   逐字母顺序相等即命中，空白随替换一并删除）；
/// - 整词边界：错误词为纯 ASCII 字母 / 数字时，要求前后相邻字符不是 ASCII 字母 /
///   数字（避免 "F2" 规则错改 "F22"）；中文错误词以字为词，无边界要求；
/// - 正确词为空 = 删除该词；单趟扫描、命中即前进，规则不互相级联。
pub fn apply_replace_rules(text: &str, rules: &[ReplaceRule]) -> String {
    if text.is_empty() || rules.is_empty() {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'outer: while i < chars.len() {
        for rule in rules {
            let wrong: Vec<char> = rule.wrong.chars().collect();
            if wrong.is_empty() {
                continue;
            }
            // 命中长度（纯 ASCII 错误词走宽松匹配，空白穿插不固定长度）。
            let consumed = if is_ascii_word(&wrong) {
                match match_ascii_word_loose(&chars, i, &wrong) {
                    Some(consumed) => consumed,
                    None => continue,
                }
            } else {
                if i + wrong.len() > chars.len() {
                    continue;
                }
                if !chars[i..i + wrong.len()]
                    .iter()
                    .zip(wrong.iter())
                    .all(|(a, b)| a.eq_ignore_ascii_case(b))
                {
                    continue;
                }
                wrong.len()
            };
            if is_ascii_word(&wrong) {
                let before_ok = i == 0 || !chars[i - 1].is_ascii_alphanumeric();
                let after = i + consumed;
                let after_ok = after >= chars.len() || !chars[after].is_ascii_alphanumeric();
                if !before_ok || !after_ok {
                    continue;
                }
            }
            out.push_str(&rule.right); // 空 right = 删除该词
            i += consumed;
            continue 'outer;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 纯 ASCII 错误词的宽松匹配：文本中拉丁字母之间允许穿插空白
/// （ASR 插空格修正），逐字母顺序相等（大小写不敏感）即命中，
/// 返回从 `start` 起消耗的字符数（含穿插的空白）。
fn match_ascii_word_loose(chars: &[char], start: usize, wrong: &[char]) -> Option<usize> {
    let mut ti = start;
    for (index, wc) in wrong.iter().enumerate() {
        // 首字母严格在 start 位置命中；其后每个字母前允许跳过文本中的空白。
        if index > 0 {
            while ti < chars.len() && chars[ti].is_whitespace() {
                ti += 1;
            }
        }
        if ti >= chars.len() || !chars[ti].eq_ignore_ascii_case(wc) {
            return None;
        }
        ti += 1;
    }
    Some(ti - start)
}

/// 错误词是否纯 ASCII 字母 / 数字（决定是否需要整词边界）。
fn is_ascii_word(chars: &[char]) -> bool {
    !chars.is_empty() && chars.iter().all(|c| c.is_ascii_alphanumeric())
}

/// 后处理管线：消费 `Recognized` → 按序执行启用 Stage → 广播出口事件。
pub struct PostProcess {
    /// 过滤语气词（F-SET-02，PRD 3.8.3；管线首位）。
    filter_fillers: FilterFillersStage,
    /// 智能数字转换（F-SET-02；在词典替换前，先规范数字再匹配规则）。
    smart_number: SmartNumberStage,
    /// v0.1 替换词典。
    replace: ReplaceStage,
    /// 输出繁体中文（F-SET-02；在词典替换后，正确词先落位再统一繁化）。
    traditional: TraditionalChineseStage,
    /// 删除结尾标点（F-SET-02；管线末位）。
    remove_trailing: RemoveTrailingPunctStage,
    /// 后续 Stage 注册处（v0.5 口令 / AI Stage）。
    extra: Vec<Box<dyn Stage>>,
    events: broadcast::Sender<AppEvent>,
    /// 会话代次追踪（T-010）：上屏前最后检查，已取消 / 过期会话的管线出口直接丢弃。
    tracker: Arc<SessionTracker>,
}

impl PostProcess {
    pub fn new(
        dict: DictService,
        config: ConfigStore,
        events: broadcast::Sender<AppEvent>,
        tracker: Arc<SessionTracker>,
    ) -> Arc<Self> {
        let process = Self {
            filter_fillers: FilterFillersStage::new(config.clone()),
            smart_number: SmartNumberStage::new(config.clone()),
            traditional: TraditionalChineseStage::new(config.clone()),
            remove_trailing: RemoveTrailingPunctStage::new(config.clone()),
            replace: ReplaceStage::new(dict, config),
            extra: Vec::new(),
            events,
            tracker,
        };
        let process = Arc::new(process);
        // 初始缓存：同步阻塞等待首个快照（启动时一次）。
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let process_clone = process.clone();
            tokio::spawn(async move {
                process_clone.refresh_stages().await;
            });
            let _ = handle;
        }
        process
    }

    /// 有序 Stage 列表：过滤语气词 → 智能数字 → 替换词典 → 繁体 → 删结尾标点，
    /// v0.5 Stage 按注册顺序追加。
    fn stages(&self) -> Vec<&dyn Stage> {
        let mut stages: Vec<&dyn Stage> = vec![
            &self.filter_fillers,
            &self.smart_number,
            &self.replace,
            &self.traditional,
            &self.remove_trailing,
        ];
        for stage in &self.extra {
            stages.push(stage.as_ref());
        }
        stages
    }

    /// 刷新全部 Stage 的缓存（词典 / 输出配置变更时）。
    async fn refresh_stages(&self) {
        self.filter_fillers.refresh().await;
        self.smart_number.refresh().await;
        self.replace.refresh().await;
        self.traditional.refresh().await;
        self.remove_trailing.refresh().await;
    }

    /// 启动管线循环：消费 Recognized → 管线 → PipelineOutput + recording:result；
    /// 同时监听词典 / 输出配置变更刷新 Stage 缓存。
    pub fn start(self: Arc<Self>, mut events_rx: broadcast::Receiver<AppEvent>) {
        tokio::spawn(async move {
            loop {
                match events_rx.recv().await {
                    Ok(AppEvent::Recognized { session_id, text }) => {
                        self.process_recognized(session_id, text).await;
                    }
                    Ok(AppEvent::DictChanged) => {
                        self.refresh_stages().await;
                    }
                    Ok(AppEvent::ConfigChanged { section }) => {
                        // 替换词典开关 → dict 节；四个输出选项开关 → output 节。
                        if matches!(section, ConfigSection::Dict | ConfigSection::Output) {
                            self.refresh_stages().await;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });
    }

    /// 对单条识别文本执行管线并广播出口。
    async fn process_recognized(&self, session_id: u64, text: String) {
        // T-010 管线出口检查：会话已取消或已过期（识别期间开始了新会话），
        // 结果丢弃——保证无效结果不会到达上屏（T-012 编排层上屏前还会最后检查一次）。
        if !self.tracker.is_valid(session_id) {
            info!(session_id, "会话已取消或过期，管线出口丢弃（不上屏）");
            // 会话在此截断：广播终态让 AudioService 状态机收尾（T-012）。
            let _ = self.events.send(AppEvent::SessionFinished { session_id });
            return;
        }
        let mut ctx = PipelineContext::new(session_id, text);
        for stage in self.stages() {
            if !stage.enabled() {
                continue; // 开关关闭：该 Stage 跳过（原文直出语义）
            }
            stage.process(&mut ctx);
            if ctx.short_circuit {
                break; // 短路：后续 Stage 不执行（v0.5 AI Stage 预留）
            }
        }
        // 管线出口（内部事件，T-012 PipelineService 消费：上屏 + 结果反馈 + 埋点）。
        let _ = self.events.send(AppEvent::PipelineOutput {
            session_id,
            text: ctx.text,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::dict::ReplaceRule;

    fn rule(id: u64, wrong: &str, right: &str) -> ReplaceRule {
        ReplaceRule {
            id,
            wrong: wrong.to_string(),
            right: right.to_string(),
            enabled: true,
        }
    }

    /// 规则按错误词长度降序排序（长词优先，与 DictService::enabled_snapshot 一致）。
    fn sorted(mut rules: Vec<ReplaceRule>) -> Vec<ReplaceRule> {
        rules.sort_by_key(|rule| std::cmp::Reverse(rule.wrong.chars().count()));
        rules
    }

    #[test]
    fn prd_acceptance_f2() {
        // PRD 3.4 验收用例：说“按 f二 重命名”上屏为“按 F2 重命名”。
        let rules = sorted(vec![rule(1, "f二", "F2")]);
        assert_eq!(apply_replace_rules("按f二重命名", &rules), "按F2重命名");
        assert_eq!(apply_replace_rules("按 f二 重命名", &rules), "按 F2 重命名");
    }

    #[test]
    fn prd_acceptance_email() {
        // PRD 3.4 验收用例：说“我的邮箱”上屏完整邮箱地址。
        let rules = sorted(vec![rule(1, "我的邮箱", "dev@example.com")]);
        assert_eq!(apply_replace_rules("我的邮箱", &rules), "dev@example.com");
    }

    #[test]
    fn long_word_priority() {
        // 多条规则可能同时命中时，较长的错误词优先（PRD 3.4.1 业务规则 2）。
        let rules = sorted(vec![
            rule(1, "洗衣机", "袭击"),
            rule(2, "洗衣机型号", "型号 X"),
        ]);
        assert_eq!(
            apply_replace_rules("这台洗衣机型号不错", &rules),
            "这台型号 X不错"
        );
        assert_eq!(
            apply_replace_rules("这台洗衣机坏了", &rules),
            "这台袭击坏了"
        );
    }

    #[test]
    fn english_case_insensitive() {
        let rules = sorted(vec![rule(1, "F二", "F2")]);
        assert_eq!(apply_replace_rules("按f二重命名", &rules), "按F2重命名");
        assert_eq!(apply_replace_rules("按F二重命名", &rules), "按F2重命名");
    }

    #[test]
    fn ascii_word_boundary() {
        // 完整匹配：F2 规则不改 F22 / AF2（纯 ASCII 错误词要求整词边界）。
        let rules = sorted(vec![rule(1, "F2", "二级")]);
        assert_eq!(apply_replace_rules("按F2键", &rules), "按二级键");
        assert_eq!(apply_replace_rules("按F22键", &rules), "按F22键");
        assert_eq!(apply_replace_rules("按AF2键", &rules), "按AF2键");
        assert_eq!(apply_replace_rules("F2", &rules), "二级");
    }

    #[test]
    fn delete_semantics_empty_right() {
        // 正确词为空 = 删除该词（PRD 3.4.1 业务规则 3）。
        let rules = sorted(vec![rule(1, "嗯", "")]);
        assert_eq!(apply_replace_rules("嗯好的", &rules), "好的");
    }

    #[test]
    fn no_cascade_within_single_pass() {
        // 单趟扫描：规则不互相级联（甲→乙 不会在同一趟再被 乙→丙 改写）。
        let rules = sorted(vec![rule(1, "甲", "乙"), rule(2, "乙", "丙")]);
        assert_eq!(apply_replace_rules("甲", &rules), "乙");
        assert_eq!(apply_replace_rules("甲乙", &rules), "乙丙");
    }

    #[test]
    fn multiline_replacement() {
        let long_text = "第一行\n第二行";
        let rules = sorted(vec![rule(1, "第二行", long_text)]);
        assert_eq!(
            apply_replace_rules("一二第二行三", &rules),
            "一二第一行\n第二行三"
        );
    }

    #[test]
    fn empty_inputs() {
        let rules = sorted(vec![rule(1, "a", "b")]);
        assert_eq!(apply_replace_rules("", &rules), "");
        assert_eq!(apply_replace_rules("abc", &[]), "abc");
    }

    #[test]
    fn chinese_no_boundary_requirement() {
        // 中文错误词无边界要求：以字为词，逐字完整匹配。
        let rules = sorted(vec![rule(1, "行", "hang")]);
        assert_eq!(apply_replace_rules("执行", &rules), "执hang");
    }

    #[test]
    fn latin_word_tolerates_asr_spaces() {
        // 拼音口令空格修正：ASR 对拉丁字母插空格（逐字母或按音节）仍命中。
        let rules = sorted(vec![rule(1, "haofan", "好烦")]);
        assert_eq!(apply_replace_rules("h a o f a n", &rules), "好烦");
        assert_eq!(apply_replace_rules("hao fan", &rules), "好烦");
        assert_eq!(apply_replace_rules("haofan", &rules), "好烦");
        // 句中位置同样生效。
        assert_eq!(apply_replace_rules("我说hao fan啊", &rules), "我说好烦啊");
    }

    #[test]
    fn latin_word_space_fix_keeps_boundary() {
        // 边界保护不因宽松匹配失效：错改 "haofans" / "ahaofan" 仍被阻止。
        let rules = sorted(vec![rule(1, "haofan", "好烦")]);
        assert_eq!(apply_replace_rules("haofans", &rules), "haofans");
        assert_eq!(apply_replace_rules("ahaofan", &rules), "ahaofan");
    }

    #[test]
    fn latin_space_fix_no_partial_match() {
        // 字母序列必须完整相等（"h a o f a" 缺尾字母不命中）。
        let rules = sorted(vec![rule(1, "haofan", "好烦")]);
        assert_eq!(apply_replace_rules("h a o f a", &rules), "h a o f a");
    }

    #[test]
    fn chinese_source_rule_unaffected_by_space_fix() {
        // 中文错误词逻辑不变："微信" → WeChat 不得回归。
        let rules = sorted(vec![rule(1, "微信", "WeChat")]);
        assert_eq!(apply_replace_rules("加我微信吧", &rules), "加我WeChat吧");
        assert_eq!(apply_replace_rules("微信", &rules), "WeChat");
    }

    // ---- 管线框架：顺序、开关、短路（SPEC 验收：注册两个假 Stage 验证顺序） ----

    struct FakeStage {
        name: &'static str,
        enabled: bool,
        suffix: &'static str,
        short_circuit: bool,
    }

    impl Stage for FakeStage {
        fn name(&self) -> &'static str {
            self.name
        }

        fn enabled(&self) -> bool {
            self.enabled
        }

        fn process(&self, ctx: &mut PipelineContext) {
            ctx.text.push_str(self.suffix);
            if self.short_circuit {
                ctx.short_circuit = true;
            }
        }
    }

    /// 管线执行骨架（与 PostProcess::process_recognized 同构）。
    fn run_pipeline(stages: &[&dyn Stage], ctx: &mut PipelineContext) {
        for stage in stages {
            if !stage.enabled() {
                continue;
            }
            stage.process(ctx);
            if ctx.short_circuit {
                break;
            }
        }
    }

    #[test]
    fn pipeline_runs_stages_in_registration_order() {
        let a = FakeStage {
            name: "a",
            enabled: true,
            suffix: "1",
            short_circuit: false,
        };
        let b = FakeStage {
            name: "b",
            enabled: true,
            suffix: "2",
            short_circuit: false,
        };
        let mut ctx = PipelineContext::new(1, "x".to_string());
        run_pipeline(&[&a, &b], &mut ctx);
        assert_eq!(ctx.text, "x12", "Stage 应按注册顺序执行");
    }

    #[test]
    fn pipeline_skips_disabled_stage() {
        let a = FakeStage {
            name: "a",
            enabled: true,
            suffix: "1",
            short_circuit: false,
        };
        let b = FakeStage {
            name: "b",
            enabled: false,
            suffix: "2",
            short_circuit: false,
        };
        let mut ctx = PipelineContext::new(1, "x".to_string());
        run_pipeline(&[&a, &b], &mut ctx);
        assert_eq!(ctx.text, "x1", "关闭的 Stage 应跳过（原文直出语义）");
    }

    #[test]
    fn pipeline_short_circuit_stops_following_stages() {
        let a = FakeStage {
            name: "a",
            enabled: true,
            suffix: "1",
            short_circuit: true,
        };
        let b = FakeStage {
            name: "b",
            enabled: true,
            suffix: "2",
            short_circuit: false,
        };
        let mut ctx = PipelineContext::new(1, "x".to_string());
        run_pipeline(&[&a, &b], &mut ctx);
        assert_eq!(
            ctx.text, "x1",
            "短路后后续 Stage 不再执行（v0.5 AI Stage 预留）"
        );
    }
}
