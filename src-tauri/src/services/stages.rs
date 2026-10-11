//! 文本输出选项 Stage（F-SET-02、PRD 3.8.3）：过滤语气词 / 智能数字转换 /
//! 输出繁体中文 / 删除结尾标点。
//!
//! - 开关绑定 `OutputConfig` 对应字段（产品原则 3：自动改写可关闭），
//!   缓存快照 + `refresh` 异步刷新 + `config:changed(output)` 事件驱动，
//!   模式与 `ReplaceStage` 一致（`services/post_process.rs`）；
//! - 全部实现为纯函数 + Stage 薄封装，纯函数单测逐条覆盖验收用例；
//! - 保守优先（宁可漏转不可错转）：识别不了的串原样保留，绝不产出空文本。
//!
//! 隐私（NFR-S-02）：日志只记录 Stage 名与是否发生改写，绝不记录文本内容。

use std::sync::Mutex;

use tracing::info;

use crate::services::config_store::ConfigStore;
use crate::services::post_process::{PipelineContext, Stage};

/// 声明一个输出选项 Stage：结构体 + 构造 / 开关缓存刷新 + Stage 实现。
/// `$transform` 为纯函数（`&str -> String`），Stage 仅做薄封装与改写日志。
macro_rules! output_flag_stage {
    ($stage:ident, $field:ident, $stage_name:literal, $transform:path) => {
        pub struct $stage {
            config: ConfigStore,
            cache: Mutex<bool>,
        }

        impl $stage {
            pub fn new(config: ConfigStore) -> Self {
                Self {
                    config,
                    cache: Mutex::new(false),
                }
            }

            /// 异步刷新开关缓存（`config:changed(output)` 时调用）。
            pub async fn refresh(&self) {
                let enabled = self.config.get().await.output.$field;
                if let Ok(mut cache) = self.cache.lock() {
                    *cache = enabled;
                }
            }
        }

        impl Stage for $stage {
            fn name(&self) -> &'static str {
                $stage_name
            }

            fn enabled(&self) -> bool {
                self.cache.lock().map(|cache| *cache).unwrap_or(false)
            }

            fn process(&self, ctx: &mut PipelineContext) {
                let before = ctx.text.clone();
                ctx.text = $transform(&ctx.text);
                if ctx.text != before {
                    info!(stage = %self.name(), "输出选项已修正识别文本");
                }
            }
        }
    };
}

// ---- 过滤语气词（PRD 3.8.3：自动删除"嗯""啊"等语气词；保守：宁可漏删不可错删） ----

/// 句首可连续删除的填充字。
const LEADING_FILLERS: [char; 6] = ['嗯', '啊', '呃', '额', '哦', '欸'];
/// 句尾可删除的单个语气词。
const TAIL_PARTICLES: [char; 6] = ['啊', '呀', '呢', '吧', '嘛', '哦'];
/// 常见重复填充词：相邻重复时删一份（"那个那个" → "那个"）。
const REPEAT_FILLERS: [&str; 5] = ["那个", "就是", "然后", "其实", "这个"];
/// 句尾语气词删除要求的前文最小长度（避免"好啊"被误删为"好"）。
const TAIL_MIN_PREFIX_LEN: usize = 2;

output_flag_stage!(
    FilterFillersStage,
    filter_fillers,
    "filter_fillers",
    filter_fillers
);

/// 保守的语气词过滤（纯函数）：
/// 1. 删除句首连续的填充字（嗯/啊/呃/额/哦/欸）；
/// 2. 删除相邻重复的常见填充词（那个那个 → 那个）；
/// 3. 删除句尾单个语气词，但要求前文至少 2 字（"好啊"保持原样）；
/// 4. 若过滤结果为空，保留原文（绝不把短文本删光）。
pub fn filter_fillers(text: &str) -> String {
    let mut chars: Vec<char> = text.chars().collect();

    // 1. 句首连续填充字。
    let mut start = 0;
    while start < chars.len() && LEADING_FILLERS.contains(&chars[start]) {
        start += 1;
    }
    if start > 0 {
        chars.drain(..start);
    }

    // 2. 相邻重复填充词（允许两份之间最多一个空白字符）。
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let mut matched = false;
        for filler in REPEAT_FILLERS {
            let fc: Vec<char> = filler.chars().collect();
            // 第一份 + 可选空白 + 第二份。
            if i + fc.len() * 2 > chars.len() {
                continue;
            }
            if chars[i..i + fc.len()] != fc[..] {
                continue;
            }
            let mut second = i + fc.len();
            if second < chars.len() && chars[second].is_whitespace() {
                second += 1;
            }
            if second + fc.len() <= chars.len() && chars[second..second + fc.len()] == fc[..] {
                out.extend_from_slice(&fc);
                i = second + fc.len();
                matched = true;
                break;
            }
        }
        if !matched {
            out.push(chars[i]);
            i += 1;
        }
    }

    // 3. 句尾单个语气词（前文至少 2 字才删）。
    if out.len() > TAIL_MIN_PREFIX_LEN && out.last().is_some_and(|c| TAIL_PARTICLES.contains(c)) {
        out.pop();
    }

    // 4. 保底：不产出空文本。
    if out.is_empty() {
        return text.to_string();
    }
    out.into_iter().collect()
}

// ---- 智能数字转换（PRD 3.8.3："一百二十" → "120"；保守：识别不了的串原样保留） ----

output_flag_stage!(
    SmartNumberStage,
    smart_number,
    "smart_number",
    apply_smart_number
);

/// 中文数字字符 → 数值（零一二三四五六七八九两）。
fn cn_digit(c: char) -> Option<u64> {
    match c {
        '零' => Some(0),
        '一' => Some(1),
        '二' | '两' => Some(2),
        '三' => Some(3),
        '四' => Some(4),
        '五' => Some(5),
        '六' => Some(6),
        '七' => Some(7),
        '八' => Some(8),
        '九' => Some(9),
        _ => None,
    }
}

/// 单位字符 → 倍率（十百千万亿）。
fn cn_unit(c: char) -> Option<u64> {
    match c {
        '十' => Some(10),
        '百' => Some(100),
        '千' => Some(1000),
        '万' => Some(10_000),
        '亿' => Some(100_000_000),
        _ => None,
    }
}

/// 解析一整段中文数字串（须完整由数字字符与单位组成，否则返回 None）。
///
/// 分段累进：数字 + 单位（十百千）累积到节；万 / 亿提升节并汇总。
/// "一百二十" → 120，"一千零五" → 1005，"十二" → 12，"十" → 10。
fn parse_cn_number(run: &[char]) -> Option<u64> {
    if run.is_empty() {
        return None;
    }
    let mut total: u64 = 0; // 万 / 亿已汇总部分
    let mut section: u64 = 0; // 当前节（万 / 亿内）
    let mut number: u64 = 0; // 最近的数字（单位前缺省视为 1，如"十"）
    let mut has_digit = false;
    for &c in run {
        if let Some(digit) = cn_digit(c) {
            number = digit;
            has_digit = true;
            continue;
        }
        let unit = cn_unit(c)?;
        if !has_digit && unit < 10_000 {
            // 单位前无数字且不是节提升（"十"视为 1×10；"十十"等怪串拒绝）。
            if section > 0 || number > 0 {
                return None;
            }
            number = 1;
        }
        if unit < 10_000 {
            section += number.saturating_mul(unit);
            number = 0;
        } else {
            // 万 / 亿：提升当前节。
            let section_total = section.saturating_add(number);
            if section_total == 0 {
                return None; // "万"前无数（"亿万"等怪串拒绝）。
            }
            section = section_total.saturating_mul(unit);
            total = total.saturating_add(section);
            section = 0;
            number = 0;
        }
    }
    if !has_digit {
        return None; // 纯单位串（"十"）不转换，保守保留。
    }
    Some(total.saturating_add(section).saturating_add(number))
}

/// 智能数字转换（纯函数）：扫描文本中的中文数字串，包含单位且可完整
/// 解析的串转为阿拉伯数字；其余原样保留（含纯单位串与单个数字字）。
pub fn apply_smart_number(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        // 收集最长的数字字符 / 单位连续串。
        if cn_digit(chars[i]).is_none() && cn_unit(chars[i]).is_none() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let mut end = i;
        while end < chars.len() && (cn_digit(chars[end]).is_some() || cn_unit(chars[end]).is_some())
        {
            end += 1;
        }
        let run = &chars[i..end];
        // 保守：仅转换包含单位（十百千万亿）的串，单数字字（"一"）不转换。
        let has_unit = run.iter().any(|c| cn_unit(*c).is_some());
        let converted = if has_unit {
            parse_cn_number(run)
                .filter(|value| *value > 0)
                .map(|value| value.to_string())
        } else {
            None
        };
        match converted {
            Some(digits) => out.push_str(&digits),
            None => out.extend(run.iter()),
        }
        i = end;
    }
    out
}

// ---- 输出繁体中文（PRD 3.8.3；zhconv 纯 Rust OpenCC 实现，离线可用） ----

/// 简体 → 繁体（zh-Hant）转换。
fn to_traditional(text: &str) -> String {
    zhconv::zhconv(text, zhconv::Variant::ZhHant)
}

// 输出繁体中文 Stage（简体 → 繁体 zh-Hant；置于替换词典之后，
// 保证词典正确词（可能含简体）先落位再统一繁化）。
output_flag_stage!(
    TraditionalChineseStage,
    traditional_chinese,
    "traditional_chinese",
    to_traditional
);

// ---- 删除结尾标点（PRD 3.8.3：自动删除结尾标点，包括"。"和"？"） ----

/// 循环剥离的结尾标点集合（全半角）。
const TRAILING_PUNCT: [char; 14] = [
    '。', '．', '.', '？', '?', '！', '!', '，', ',', '、', '；', ';', '：', ':',
];

// 删除结尾标点 Stage（管线末位：最后执行）。
output_flag_stage!(
    RemoveTrailingPunctStage,
    remove_trailing_punct,
    "remove_trailing_punct",
    remove_trailing_punct
);

/// 循环剥离末尾标点与空白（纯函数）：末尾的空白也一并删除。
pub fn remove_trailing_punct(text: &str) -> String {
    let mut end = text.len();
    loop {
        let trimmed = text[..end].trim_end();
        let chars: Vec<char> = trimmed.chars().collect();
        let mut cut = trimmed.len();
        if let Some(last) = chars.last() {
            if TRAILING_PUNCT.contains(last) {
                cut = trimmed.len() - last.len_utf8();
            }
        }
        if cut == end {
            break;
        }
        end = cut;
    }
    text[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 过滤语气词 ----

    #[test]
    fn filler_leading_and_tail() {
        // 验收用例：句首填充串 + 句尾单个语气词均删除。
        assert_eq!(
            filter_fillers("嗯这个东西一百二十块啊"),
            "这个东西一百二十块"
        );
        assert_eq!(filter_fillers("嗯呃好的"), "好的");
    }

    #[test]
    fn filler_tail_keeps_short_prefix() {
        // 保守：前文不足 2 字不删（"好啊"不得变"好"）。
        assert_eq!(filter_fillers("好啊"), "好啊");
        assert_eq!(filter_fillers("哦"), "哦");
    }

    #[test]
    fn filler_tail_with_enough_prefix() {
        assert_eq!(filter_fillers("你好呀"), "你好");
        assert_eq!(filter_fillers("就这样吧"), "就这样");
    }

    #[test]
    fn filler_repeated_words() {
        assert_eq!(filter_fillers("那个那个"), "那个");
        assert_eq!(filter_fillers("就是就是"), "就是");
        assert_eq!(filter_fillers("然后然后"), "然后");
        assert_eq!(filter_fillers("那个 那个"), "那个");
        // 单次出现不受影响。
        assert_eq!(filter_fillers("那个东西"), "那个东西");
    }

    #[test]
    fn filler_never_produces_empty() {
        assert_eq!(filter_fillers("嗯"), "嗯");
        assert_eq!(filter_fillers("啊"), "啊");
        assert_eq!(filter_fillers("嗯啊"), "嗯啊", "删光则保留原文");
    }

    #[test]
    fn filler_mid_text_untouched() {
        // 句中语气词不删（保守）。
        assert_eq!(filter_fillers("其实啊你说的对"), "其实啊你说的对");
    }

    // ---- 智能数字转换 ----

    #[test]
    fn number_basic() {
        // PRD 3.8.3 验收用例。
        assert_eq!(apply_smart_number("一百二十"), "120");
        assert_eq!(apply_smart_number("一千零五"), "1005");
    }

    #[test]
    fn number_units_and_sections() {
        assert_eq!(apply_smart_number("十二"), "12");
        assert_eq!(apply_smart_number("一万两千三百"), "12300");
        assert_eq!(apply_smart_number("两亿零三万"), "200030000");
        assert_eq!(apply_smart_number("三百六十行"), "360行");
    }

    #[test]
    fn number_in_sentence() {
        assert_eq!(apply_smart_number("价格是一百二十块"), "价格是120块");
    }

    #[test]
    fn number_conservative_keeps_ambiguous() {
        // 单数字字不转换（避免"测试一下"→"测试1下"）。
        assert_eq!(apply_smart_number("测试一下"), "测试一下");
        // 纯单位串不转换。
        assert_eq!(apply_smart_number("十周年"), "十周年");
        // 无法解析的串原样保留。
        assert_eq!(apply_smart_number("一二三四五"), "一二三四五");
    }

    #[test]
    fn number_weird_runs_rejected() {
        // 重复节提升（亿万 / 万十）等怪串保守保留。
        assert_eq!(apply_smart_number("一亿万"), "一亿万");
    }

    // ---- 繁体转换 ----

    #[test]
    fn traditional_basic() {
        let out = zhconv::zhconv("实时语音输入会输出繁体中文", zhconv::Variant::ZhHant);
        assert_eq!(out, "實時語音輸入會輸出繁體中文");
    }

    #[test]
    fn traditional_keeps_latin_and_digits() {
        let out = zhconv::zhconv("按 F2 重命名", zhconv::Variant::ZhHant);
        assert_eq!(out, "按 F2 重命名");
    }

    // ---- 删除结尾标点 ----

    #[test]
    fn trailing_punct_full_width() {
        assert_eq!(remove_trailing_punct("好的。"), "好的");
        assert_eq!(remove_trailing_punct("在吗？"), "在吗");
        assert_eq!(remove_trailing_punct("完成了！"), "完成了");
    }

    #[test]
    fn trailing_punct_half_width_and_loop() {
        assert_eq!(remove_trailing_punct("hello."), "hello");
        assert_eq!(remove_trailing_punct("好。 "), "好");
        assert_eq!(remove_trailing_punct("好。！？ "), "好");
    }

    #[test]
    fn trailing_punct_mixed_set() {
        assert_eq!(remove_trailing_punct("字段，"), "字段");
        assert_eq!(remove_trailing_punct("字段、"), "字段");
        assert_eq!(remove_trailing_punct("字段；"), "字段");
        assert_eq!(remove_trailing_punct("字段："), "字段");
        assert_eq!(remove_trailing_punct("字段．"), "字段");
    }

    #[test]
    fn trailing_punct_keeps_inner() {
        assert_eq!(remove_trailing_punct("你好，世界。"), "你好，世界");
        assert_eq!(remove_trailing_punct("没有标点"), "没有标点");
    }
}
