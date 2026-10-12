//! 提示词模板引擎（T-533，F-AI-01/F-AI-03；SPEC: doc/specs/v0.5/T-533-prompt-templates.md）。
//!
//! - 模板数据模型：名称、Prompt 正文、绑定供应商 / 模型、快捷键（存储字段预留，
//!   绑定机制在 T-534）、超时时间（PRD 3.5.1，默认 5000 ms）、内置标记；
//!   上下文变量为 v1.0（F-AI-04）不在本模型内；
//! - 内置编程向模板（F-AI-03，PRD 3.5.2 清单，数据化目录 [`BUILTIN_CATALOG`]）：
//!   只读、可复制为自定义（SPEC T-533 验收标准；与 PRD 3.5.1 业务规则 1 的
//!   "内置可编辑 / 可删除 / 可恢复"差异已在任务报告说明，待文档对齐）；
//! - 持久化：配置目录 `templates.json`（`write_json_atomic` 原子写入，NFR-R-05）；
//!   非法 / 损坏文件回退为内置模板重建（NFR-S-07 雏形）；
//! - 导入导出校验接口预留（[`validate_import`]，NFR-S-07；备份恢复在 v1.0）。
//!
//! 安全（NFR-S-02）：模板正文（prompt）属于用户内容，**任何日志语句不得携带**；
//! 本模块日志只记录模板 ID 与计数。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::error::AppError;
use crate::services::config_store::{write_json_atomic, AiProviderEntry};

use super::presets::{ERR_AI_BAD_VALUE, ERR_AI_NOT_FOUND};

/// 模板文件名（配置目录下，tech/06 6.1.2）。
pub const TEMPLATES_FILE_NAME: &str = "templates.json";
/// 模板名称长度上限（PRD 3.5.1：最多 20 字）。
pub const MAX_NAME_CHARS: usize = 20;
/// 提示词内容长度上限（PRD 3.5.1：最多 10,000 字）。
pub const MAX_PROMPT_CHARS: usize = 10_000;
/// 超时时间下限（毫秒，PRD 3.5.1）。
pub const MIN_TIMEOUT_MS: u32 = 1_000;
/// 超时时间上限（毫秒，PRD 3.5.1）。
pub const MAX_TIMEOUT_MS: u32 = 60_000;
/// 超时时间默认值（毫秒，PRD 3.5.1）。
pub const DEFAULT_TIMEOUT_MS: u32 = 5_000;
/// 快捷键键名长度上限（存储字段预留 T-534，仅作输入防呆）。
const MAX_HOTKEY_CHARS: usize = 128;

/// 错误编号：模板文件读写失败（与 ConfigStore `ERR_IO` 同号，E-CONFIG-001）。
const ERR_IO: u32 = 1;

/// AI 提示词模板（tech/06 6.1.2 基础结构 + PRD 3.5.1 编辑字段）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiTemplate {
    /// 稳定 ID（内置为固定标识如 `correction`；自定义为生成 ID `tpl-<hex>`）。
    pub id: String,
    /// 模板名称（最多 20 字，全局唯一）。
    pub name: String,
    /// Prompt 正文（最多 10,000 字）。隐私：不写入日志（NFR-S-02）。
    pub prompt: String,
    /// 绑定供应商 ID（None = 无（纯转录））。
    pub provider_id: Option<String>,
    /// 绑定模型标识（选择供应商后必填；须来自该供应商的模型清单）。
    pub model: Option<String>,
    /// 快捷键（存储字段预留；绑定生效机制在 T-534）。
    pub hotkey: Option<String>,
    /// 超时时间（毫秒，1000–60000；AI 超时降级在 T-535）。
    pub timeout_ms: u32,
    /// 内置模板标记（只读、可复制，不可修改 / 删除）。
    pub builtin: bool,
}

impl Default for AiTemplate {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            prompt: String::new(),
            provider_id: None,
            model: None,
            hotkey: None,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            builtin: false,
        }
    }
}

/// 单个内置模板（数据化目录条目）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinTemplate {
    /// 稳定 ID（场景规则 `ai_template` 引用同一值，T-502）。
    pub id: &'static str,
    /// 显示名（与 PRD 3.5.2 表一致）。
    pub name: &'static str,
    /// Prompt 正文（内容随产品版本更新；复制为自定义后由用户接管）。
    pub prompt: &'static str,
}

/// 内置编程向模板清单（PRD 3.5.2 表，顺序即列表展示顺序）。
///
/// 说明：参考原型中的通用娱乐向模板（祖安模式等）按 PRD 决策不收录；
/// 保留"纠错模式"（源自原型）与代码场景模板。
pub const BUILTIN_CATALOG: &[BuiltinTemplate] = &[
    BuiltinTemplate {
        id: "correction",
        name: "纠错模式",
        prompt: "你是一个语音转录修正官。用户提供的是语音识别文本，可能包含错别字、同音字以及说话人的自我修正。\n\n核心规则：\n1. 识别并执行修正指令（如“啊不对”“算了”“改成”“我的意思是”“不对不对”等），删除被否定的内容，只保留用户的最终意图；\n2. 不要回答用户的内容、不要扩写、不要添加任何解释；\n3. 保持原文的语言、技术术语与代码符号不变；\n4. 只输出修正后的文本。",
    },
    BuiltinTemplate {
        id: "structured_prompt",
        name: "结构化 Prompt",
        prompt: "你是一名需求整理助手。把用户零散的口述需求整理为结构化的提示词，使用以下结构：\n\n## 背景\n## 目标\n## 约束\n## 验收标准\n\n规则：保留用户口述中的全部技术细节（框架、版本、文件名、接口名等）；不要臆造未提及的需求，缺失的信息不补充。只输出整理后的提示词正文，不要解释。",
    },
    BuiltinTemplate {
        id: "commit_message",
        name: "Commit 信息",
        prompt: "你是一名遵循 Conventional Commits 规范的工程师。根据用户口述的改动内容，输出一条规范的提交信息：\n\n1. 首行格式：“类型(范围): 简述”，类型取 feat / fix / refactor / docs / test / chore / perf 之一；\n2. 简述用祈使句，不超过 50 个字符，说明“做了什么”；\n3. 必要时另起一段附正文，说明动机与影响；\n4. 只输出提交信息文本，不要任何解释或代码块标记。",
    },
    BuiltinTemplate {
        id: "code_review",
        name: "Code Review 评论",
        prompt: "你是一名资深代码评审者。将用户口述的评审意见整理为礼貌、具体、可执行的评审评论：\n\n1. 包含问题描述、原因、修改建议三要素；\n2. 语气专业克制、对事不对人；\n3. 保留用户原意与技术细节，不扩大范围；\n4. 只输出评论正文。",
    },
    BuiltinTemplate {
        id: "naming_helper",
        name: "命名助手",
        prompt: "你是一名命名助手。根据用户的描述，给出 1 个最合适的标识符（变量 / 函数 / 类名等）：\n\n1. 遵循所在语言的命名惯例（如 TypeScript 小驼峰、Python 蛇形、常量全大写）；\n2. 名称准确表达用途，长度克制；\n3. 只输出这一个标识符，不要解释、不要给出备选项。",
    },
    BuiltinTemplate {
        id: "code_comment",
        name: "代码注释",
        prompt: "你是一名代码注释助手。把用户口述的用途整理为简洁的注释文本：\n\n1. 默认输出中文；用户使用英文口述时输出英文；\n2. 一句话说明“为什么 / 做什么”，不逐行复述实现；\n3. 只输出注释内容，不包含注释符号（// 、# 等），不加解释。",
    },
    BuiltinTemplate {
        id: "tech_translate",
        name: "技术翻译",
        prompt: "你是一名技术翻译。按输入语言反向翻译（中文口述 → 地道的英文技术表达；英文口述 → 中文）：\n\n1. 代码、命令、标识符、术语、链接、版本号保持原样不翻译；\n2. 使用目标语言技术社区的自然表达，不逐字直译；\n3. 不增删信息、不加解释；\n4. 只输出译文。",
    },
    BuiltinTemplate {
        id: "terminal_command",
        name: "终端命令",
        prompt: "你是一名终端助手。根据用户口述的操作，输出一条对应当前操作系统的命令：\n\n1. 只输出命令本身（可含必要参数），不要解释；\n2. 操作在不同系统下命令不同时，优先给出 macOS / Linux 命令，Windows 用 PowerShell 等价命令，各以一行注释标明系统；\n3. 命令具有破坏性或高风险（删除、覆盖、权限提升、格式化等）时，在命令后另起一行以“⚠ 提醒：”附一句简短的中文风险说明。",
    },
    BuiltinTemplate {
        id: "tech_doc",
        name: "技术文档",
        prompt: "你是一名技术文档助手。把用户口述的说明整理为结构清晰的 Markdown 段落：\n\n1. 使用标题与列表组织内容，层级不超过三级；\n2. 去口语化、去重复，保留全部技术信息与数字；\n3. 不添加未提及的内容、不臆造示例；\n4. 只输出 Markdown 正文。",
    },
];

impl BuiltinTemplate {
    /// 目录条目 → 模板（内置标记；默认不绑定供应商，显示“纯转录”，PRD 3.5.2）。
    pub fn to_template(&self) -> AiTemplate {
        AiTemplate {
            id: self.id.to_string(),
            name: self.name.to_string(),
            prompt: self.prompt.to_string(),
            provider_id: None,
            model: None,
            hotkey: None,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            builtin: true,
        }
    }
}

/// 按 ID 查找内置模板目录条目。
pub fn find_builtin(id: &str) -> Option<&'static BuiltinTemplate> {
    BUILTIN_CATALOG.iter().find(|builtin| builtin.id == id)
}

/// 生成自定义模板 ID（`tpl-` + 16 位随机 hex）。
fn new_template_id() -> String {
    let bytes: [u8; 8] = rand::random();
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("tpl-{hex}")
}

fn bad_value(message: impl Into<String>) -> AppError {
    AppError::Config(ERR_AI_BAD_VALUE, message.into())
}

fn not_found(message: impl Into<String>) -> AppError {
    AppError::Config(ERR_AI_NOT_FOUND, message.into())
}

/// 可选字符串规整：去首尾空白，空串归一为 None。
fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// 内置模板并入清单：目录内内置条目按目录顺序置于最前并以目录内容刷新
/// （内置只读且随版本更新）；未知内置（未来版本写入）与非内置随时序保留。
/// 返回清单是否发生变化。
fn merge_builtins(templates: &mut Vec<AiTemplate>) -> bool {
    let before = templates.clone();
    let mut unknown_builtins: Vec<AiTemplate> = Vec::new();
    let mut customs: Vec<AiTemplate> = Vec::new();
    for template in templates.drain(..) {
        if template.builtin {
            if find_builtin(&template.id).is_none() {
                unknown_builtins.push(template);
            }
            // 目录内的内置条目丢弃重建：内容以当前版本的目录为准。
        } else {
            customs.push(template);
        }
    }
    let mut merged: Vec<AiTemplate> = BUILTIN_CATALOG.iter().map(|b| b.to_template()).collect();
    merged.append(&mut unknown_builtins);
    merged.append(&mut customs);
    *templates = merged;
    *templates != before
}

/// 校验并规整模板输入（创建 / 更新共用）。
///
/// `others` 为除自身外的全部模板（名称唯一性）；`providers` 为当前供应商清单
/// （绑定有效性：供应商存在、模型必填且来自其模型清单）。
fn normalize_and_validate(
    template: &mut AiTemplate,
    others: &[AiTemplate],
    providers: &[AiProviderEntry],
) -> Result<(), AppError> {
    template.name = template.name.trim().to_string();
    if template.name.is_empty() {
        return Err(bad_value("模板名称不能为空"));
    }
    if template.name.chars().count() > MAX_NAME_CHARS {
        return Err(bad_value(format!("模板名称最多 {MAX_NAME_CHARS} 字")));
    }
    if others.iter().any(|other| other.name == template.name) {
        return Err(bad_value(format!(
            "模板名称“{}”与已有模板重复",
            template.name
        )));
    }
    template.prompt = template.prompt.trim().to_string();
    if template.prompt.is_empty() {
        return Err(bad_value("提示词内容不能为空"));
    }
    if template.prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(bad_value(format!("提示词内容最多 {MAX_PROMPT_CHARS} 字")));
    }
    // 快捷键（存储字段预留 T-534）：仅长度防呆，冲突检测归 T-534 / T-543。
    template.hotkey = normalize_optional(template.hotkey.take());
    if let Some(hotkey) = &template.hotkey {
        if hotkey.chars().count() > MAX_HOTKEY_CHARS {
            return Err(bad_value("快捷键键名过长"));
        }
    }
    if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&template.timeout_ms) {
        return Err(bad_value(format!(
            "超时时间须在 {MIN_TIMEOUT_MS}–{MAX_TIMEOUT_MS} 毫秒之间"
        )));
    }
    // 绑定校验（PRD 3.5.1：选择供应商后模型必填；模型来自该供应商模型列表）。
    template.provider_id = normalize_optional(template.provider_id.take());
    template.model = normalize_optional(template.model.take());
    match &template.provider_id {
        None => {
            if template.model.is_some() {
                return Err(bad_value("未选择 AI 供应商时不能指定模型"));
            }
        }
        Some(provider_id) => {
            let provider = providers
                .iter()
                .find(|entry| &entry.id == provider_id)
                .ok_or_else(|| bad_value(format!("AI 供应商不存在（{provider_id}）")))?;
            let Some(model) = template.model.as_deref() else {
                return Err(bad_value("已选择 AI 供应商，请同时选择模型"));
            };
            if !provider.available_models.iter().any(|id| id == model) {
                return Err(bad_value(format!(
                    "模型“{model}”不在供应商“{}”的模型列表中，请先在 AI 供应商页拉取",
                    provider.name
                )));
            }
        }
    }
    Ok(())
}

/// 生成不与现有模板重复的副本名（名称上限内自动截断与加序号）。
fn unique_copy_name(base: &str, templates: &[AiTemplate]) -> String {
    let taken = |name: &str| templates.iter().any(|template| template.name == name);
    let mut index = 1u32;
    loop {
        let suffix = if index == 1 {
            "（副本）".to_string()
        } else {
            format!("（副本 {index}）")
        };
        let base_limit = MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
        let trimmed: String = base.trim().chars().take(base_limit).collect();
        let candidate = format!("{}{suffix}", trimmed.trim_end());
        if !taken(&candidate) {
            return candidate;
        }
        index += 1;
    }
}

/// 提示词模板服务：`templates.json` 的内存镜像 + 原子落盘。
pub struct TemplatesService {
    path: PathBuf,
    templates: RwLock<Vec<AiTemplate>>,
}

impl TemplatesService {
    /// 加载模板文件（不存在 / 损坏时以内置模板重建并立即落盘）。
    pub fn load(config_dir: &Path) -> Result<Self, AppError> {
        let path = config_dir.join(TEMPLATES_FILE_NAME);
        let mut templates: Vec<AiTemplate> = match fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<Vec<AiTemplate>>(&content) {
                Ok(list) => list,
                Err(err) => {
                    // 隐私（NFR-S-02）：只记录解析错误，不记录文件正文。
                    warn!(error = %err, "模板文件解析失败，已重建内置模板");
                    Vec::new()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                return Err(AppError::Config(ERR_IO, format!("读取模板文件失败: {err}")));
            }
        };
        if merge_builtins(&mut templates) {
            write_json_atomic(&path, &templates)?;
            info!(count = templates.len(), "提示词模板已初始化");
        }
        Ok(Self {
            path,
            templates: RwLock::new(templates),
        })
    }

    /// 模板清单（内置在前，按目录顺序；自定义按添加顺序）。
    pub async fn list(&self) -> Vec<AiTemplate> {
        self.templates.read().await.clone()
    }

    /// 新建模板（ID 缺失自动生成；内置标记强制为 false）。
    pub async fn create(
        &self,
        providers: &[AiProviderEntry],
        mut template: AiTemplate,
    ) -> Result<AiTemplate, AppError> {
        let mut templates = self.templates.write().await;
        normalize_and_validate(&mut template, &templates, providers)?;
        if template.id.trim().is_empty() {
            template.id = new_template_id();
        } else {
            template.id = template.id.trim().to_string();
        }
        if templates.iter().any(|existing| existing.id == template.id) {
            return Err(bad_value(format!("模板 ID 已存在（{}）", template.id)));
        }
        template.builtin = false;
        templates.push(template.clone());
        self.save(&templates)?;
        // 隐私（NFR-S-02）：日志只含模板 ID，不含名称与正文。
        info!(id = %template.id, "已新建提示词模板");
        Ok(template)
    }

    /// 更新模板（整体替换）；内置模板只读（返回 E-CONFIG-004）。
    pub async fn update(
        &self,
        providers: &[AiProviderEntry],
        mut template: AiTemplate,
    ) -> Result<AiTemplate, AppError> {
        let id = template.id.trim().to_string();
        if id.is_empty() {
            return Err(bad_value("模板 ID 不能为空"));
        }
        let mut templates = self.templates.write().await;
        let index = templates
            .iter()
            .position(|existing| existing.id == id)
            .ok_or_else(|| not_found(format!("模板不存在（{id}）")))?;
        if templates[index].builtin {
            return Err(bad_value("内置模板只读，请复制为自定义模板后再修改"));
        }
        let others: Vec<AiTemplate> = templates
            .iter()
            .filter(|existing| existing.id != id)
            .cloned()
            .collect();
        normalize_and_validate(&mut template, &others, providers)?;
        template.id = id;
        template.builtin = false;
        templates[index] = template.clone();
        self.save(&templates)?;
        info!(id = %template.id, "已更新提示词模板");
        Ok(template)
    }

    /// 删除模板；内置模板不可删除（返回 E-CONFIG-004）。
    pub async fn delete(&self, id: &str) -> Result<(), AppError> {
        let mut templates = self.templates.write().await;
        let index = templates
            .iter()
            .position(|existing| existing.id == id)
            .ok_or_else(|| not_found(format!("模板不存在（{id}）")))?;
        if templates[index].builtin {
            return Err(bad_value("内置模板不可删除（只读，可复制为自定义）"));
        }
        templates.remove(index);
        self.save(&templates)?;
        info!(id = %id, "已删除提示词模板");
        Ok(())
    }

    /// 复制模板为自定义（内置模板的唯一定制路径）。
    ///
    /// 副本：新 ID、自动唯一名称、清除快捷键（防冲突，绑定归 T-534）；
    /// 供应商 / 模型绑定与其他字段原样保留。
    pub async fn copy(&self, id: &str) -> Result<AiTemplate, AppError> {
        let mut templates = self.templates.write().await;
        let source = templates
            .iter()
            .find(|existing| existing.id == id)
            .cloned()
            .ok_or_else(|| not_found(format!("模板不存在（{id}）")))?;
        let name = unique_copy_name(&source.name, &templates);
        let copy = AiTemplate {
            id: new_template_id(),
            name,
            prompt: source.prompt,
            provider_id: source.provider_id,
            model: source.model,
            hotkey: None,
            timeout_ms: source.timeout_ms,
            builtin: false,
        };
        templates.push(copy.clone());
        self.save(&templates)?;
        info!(id = %copy.id, source = %id, "已复制提示词模板");
        Ok(copy)
    }

    /// 原子落盘（NFR-R-05；调用方须持有写锁）。
    fn save(&self, templates: &[AiTemplate]) -> Result<(), AppError> {
        // write_json_atomic 的 T 为 Sized：切片经 to_vec 序列化为 JSON 数组。
        write_json_atomic(&self.path, &templates.to_vec())
    }
}

/// 校验导入内容（NFR-S-07 导入导出统一校验接口**预留**；v1.0 备份恢复使用）。
///
/// 规则：JSON 数组；逐条结构校验（名称 1–20 字且组内唯一、正文 1–10,000 字、
/// 超时 1000–60000 ms）；ID 缺失自动生成、组内重复拒绝；内置标记一律清除
/// （内置目录随版本重建）；未绑定供应商的条目清空孤立模型。
/// 不校验供应商绑定有效性（导入到其他设备时供应商可能不存在，由界面提示重新绑定）。
pub fn validate_import(json: &str) -> Result<Vec<AiTemplate>, AppError> {
    let mut list: Vec<AiTemplate> = serde_json::from_str(json)
        .map_err(|err| bad_value(format!("模板文件格式不合法: {err}")))?;
    let mut ids: HashSet<String> = HashSet::new();
    let mut names: HashSet<String> = HashSet::new();
    for (index, template) in list.iter_mut().enumerate() {
        let position = index + 1;
        template.builtin = false;
        template.id = template.id.trim().to_string();
        if template.id.is_empty() {
            template.id = new_template_id();
        }
        if !ids.insert(template.id.clone()) {
            return Err(bad_value(format!("第 {position} 条模板 ID 重复")));
        }
        template.name = template.name.trim().to_string();
        if template.name.is_empty() || template.name.chars().count() > MAX_NAME_CHARS {
            return Err(bad_value(format!(
                "第 {position} 条模板名称不合法（1–{MAX_NAME_CHARS} 字）"
            )));
        }
        if !names.insert(template.name.clone()) {
            return Err(bad_value(format!("第 {position} 条模板名称重复")));
        }
        template.prompt = template.prompt.trim().to_string();
        if template.prompt.is_empty() || template.prompt.chars().count() > MAX_PROMPT_CHARS {
            return Err(bad_value(format!(
                "第 {position} 条提示词内容不合法（1–{MAX_PROMPT_CHARS} 字）"
            )));
        }
        if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&template.timeout_ms) {
            return Err(bad_value(format!(
                "第 {position} 条超时时间超出范围（{MIN_TIMEOUT_MS}–{MAX_TIMEOUT_MS} 毫秒）"
            )));
        }
        template.provider_id = normalize_optional(template.provider_id.take());
        template.model = normalize_optional(template.model.take());
        template.hotkey = normalize_optional(template.hotkey.take());
        if template.provider_id.is_none() {
            template.model = None;
        }
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::config_store::AiProviderEntry;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "vt-ai-templates-{name}-{}-{:?}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 构造一个带模型清单的供应商（模板绑定校验用）。
    fn provider(id: &str, name: &str, models: &[&str]) -> AiProviderEntry {
        AiProviderEntry {
            id: id.to_string(),
            name: name.to_string(),
            protocol: "openai_compatible".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            available_models: models.iter().map(|model| model.to_string()).collect(),
            ..AiProviderEntry::default()
        }
    }

    fn custom(name: &str, prompt: &str) -> AiTemplate {
        AiTemplate {
            name: name.to_string(),
            prompt: prompt.to_string(),
            ..AiTemplate::default()
        }
    }

    /// 验收标准：内置模板清单与 PRD 3.5.2 表一致。
    #[test]
    fn builtin_catalog_matches_prd_list() {
        let names: Vec<_> = BUILTIN_CATALOG.iter().map(|builtin| builtin.name).collect();
        assert_eq!(
            names,
            vec![
                "纠错模式",
                "结构化 Prompt",
                "Commit 信息",
                "Code Review 评论",
                "命名助手",
                "代码注释",
                "技术翻译",
                "终端命令",
                "技术文档",
            ]
        );
        let mut ids: Vec<_> = BUILTIN_CATALOG.iter().map(|builtin| builtin.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "内置模板 ID 不得重复");
        for builtin in BUILTIN_CATALOG {
            assert!(!builtin.prompt.trim().is_empty());
            assert!(builtin.prompt.chars().count() <= MAX_PROMPT_CHARS);
            let template = builtin.to_template();
            assert!(template.builtin);
            assert_eq!(template.provider_id, None, "内置默认不绑定供应商（纯转录）");
            assert_eq!(template.id, builtin.id);
        }
    }

    /// 验收标准：首次加载灌入内置模板并持久化；重启保持、不重复。
    #[tokio::test]
    async fn load_seeds_builtins_and_persists() {
        let dir = TestDir::new("seed");
        let service = TemplatesService::load(&dir.0).unwrap();
        let list = service.list().await;
        assert_eq!(list.len(), BUILTIN_CATALOG.len());
        assert!(list.iter().all(|template| template.builtin));
        let on_disk = std::fs::read_to_string(dir.0.join(TEMPLATES_FILE_NAME)).unwrap();
        assert!(on_disk.contains("correction"));

        // 重启加载：不重复、顺序一致。
        let reloaded = TemplatesService::load(&dir.0).unwrap().list().await;
        assert_eq!(reloaded, list);
    }

    /// 验收标准：自定义模板增删改查可用，重启保持。
    #[tokio::test]
    async fn custom_crud_roundtrip_survives_reload() {
        let dir = TestDir::new("crud");
        let providers = vec![provider("deepseek", "DeepSeek", &["deepseek-chat"])];
        let service = TemplatesService::load(&dir.0).unwrap();

        // 创建（带供应商 / 模型绑定与超时）。
        let mut input = custom("我的模板", "请帮我整理：{识别文本}");
        input.provider_id = Some("deepseek".to_string());
        input.model = Some("deepseek-chat".to_string());
        input.timeout_ms = 10_000;
        let created = service.create(&providers, input).await.unwrap();
        assert!(created.id.starts_with("tpl-"));
        assert!(!created.builtin);
        assert_eq!(created.timeout_ms, 10_000);

        // 更新。
        let mut update = created.clone();
        update.name = "我的模板（改）".to_string();
        update.prompt = "新的提示词".to_string();
        update.provider_id = None;
        update.model = None;
        let updated = service.update(&providers, update).await.unwrap();
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.name, "我的模板（改）");

        // 重启保持（原子写入往返）。
        let reloaded = TemplatesService::load(&dir.0).unwrap();
        let list = reloaded.list().await;
        assert_eq!(list.len(), BUILTIN_CATALOG.len() + 1);
        let found = list
            .iter()
            .find(|template| template.id == created.id)
            .unwrap();
        assert_eq!(found.name, "我的模板（改）");
        assert_eq!(found.prompt, "新的提示词");
        assert_eq!(found.provider_id, None);

        // 删除。
        reloaded.delete(&created.id).await.unwrap();
        assert_eq!(reloaded.list().await.len(), BUILTIN_CATALOG.len());
        // 删除不存在 → E-CONFIG-005。
        assert_eq!(
            reloaded.delete("tpl-nope").await.unwrap_err().code(),
            "E-CONFIG-005"
        );
    }

    /// 验收标准：内置模板只读（不可改 / 删），可复制为自定义。
    #[tokio::test]
    async fn builtin_templates_are_read_only_but_copyable() {
        let dir = TestDir::new("builtin-guard");
        let service = TemplatesService::load(&dir.0).unwrap();
        let builtin = service
            .list()
            .await
            .into_iter()
            .find(|template| template.id == "correction")
            .unwrap();

        // 修改内置 → E-CONFIG-004；删除内置 → E-CONFIG-004。
        let mut modified = builtin.clone();
        modified.prompt = "被篡改的提示词".to_string();
        assert_eq!(
            service.update(&[], modified).await.unwrap_err().code(),
            "E-CONFIG-004"
        );
        assert_eq!(
            service.delete("correction").await.unwrap_err().code(),
            "E-CONFIG-004"
        );
        // 磁盘内容未被篡改。
        let stored = service
            .list()
            .await
            .into_iter()
            .find(|template| template.id == "correction")
            .unwrap();
        assert_eq!(stored.prompt, builtin.prompt);

        // 复制 → 新 ID、自动唯一名称、正文一致、非内置。
        let copy = service.copy("correction").await.unwrap();
        assert_ne!(copy.id, builtin.id);
        assert!(!copy.builtin);
        assert_eq!(copy.name, "纠错模式（副本）");
        assert_eq!(copy.prompt, builtin.prompt);
        // 再复制 → 序号递增。
        let copy2 = service.copy("correction").await.unwrap();
        assert_eq!(copy2.name, "纠错模式（副本 2）");

        // 副本可正常编辑与删除。
        let mut edited = copy.clone();
        edited.prompt = "定制后的提示词".to_string();
        service.update(&[], edited).await.unwrap();
        service.delete(&copy.id).await.unwrap();
    }

    /// 校验规则：名称 / 正文 / 超时 / 绑定。
    #[tokio::test]
    async fn validation_rules() {
        let dir = TestDir::new("validate");
        let providers = vec![provider("deepseek", "DeepSeek", &["deepseek-chat"])];
        let service = TemplatesService::load(&dir.0).unwrap();
        service
            .create(&providers, custom("重名", "p"))
            .await
            .unwrap();

        // 名称为空 / 超长 / 重名。
        assert_eq!(
            service
                .create(&providers, custom("  ", "p"))
                .await
                .unwrap_err()
                .code(),
            "E-CONFIG-004"
        );
        let long_name = "名".repeat(MAX_NAME_CHARS + 1);
        assert_eq!(
            service
                .create(&providers, custom(&long_name, "p"))
                .await
                .unwrap_err()
                .code(),
            "E-CONFIG-004"
        );
        assert_eq!(
            service
                .create(&providers, custom("重名", "p"))
                .await
                .unwrap_err()
                .code(),
            "E-CONFIG-004"
        );
        // 正文为空 / 超长。
        assert_eq!(
            service
                .create(&providers, custom("名字", "  "))
                .await
                .unwrap_err()
                .code(),
            "E-CONFIG-004"
        );
        let long_prompt = "p".repeat(MAX_PROMPT_CHARS + 1);
        assert_eq!(
            service
                .create(&providers, custom("名字", &long_prompt))
                .await
                .unwrap_err()
                .code(),
            "E-CONFIG-004"
        );
        // 超时范围。
        let mut out_of_range = custom("名字", "p");
        out_of_range.timeout_ms = 999;
        assert_eq!(
            service
                .create(&providers, out_of_range)
                .await
                .unwrap_err()
                .code(),
            "E-CONFIG-004"
        );
        // 绑定：供应商不存在 / 选供应商未选模型 / 模型不在清单 / 未选供应商带模型。
        let mut input = custom("名字", "p");
        input.provider_id = Some("nope".to_string());
        assert_eq!(
            service.create(&providers, input).await.unwrap_err().code(),
            "E-CONFIG-004"
        );
        let mut input = custom("名字", "p");
        input.provider_id = Some("deepseek".to_string());
        assert_eq!(
            service.create(&providers, input).await.unwrap_err().code(),
            "E-CONFIG-004"
        );
        let mut input = custom("名字", "p");
        input.provider_id = Some("deepseek".to_string());
        input.model = Some("gpt-4o".to_string());
        assert_eq!(
            service.create(&providers, input).await.unwrap_err().code(),
            "E-CONFIG-004"
        );
        let mut input = custom("名字", "p");
        input.model = Some("gpt-4o".to_string());
        assert_eq!(
            service.create(&providers, input).await.unwrap_err().code(),
            "E-CONFIG-004"
        );
        // 合法绑定通过。
        let mut input = custom("名字", "p");
        input.provider_id = Some("deepseek".to_string());
        input.model = Some("deepseek-chat".to_string());
        assert!(service.create(&providers, input).await.is_ok());
    }

    /// 损坏文件回退：以内置模板重建，不崩溃。
    #[tokio::test]
    async fn corrupted_file_falls_back_to_builtins() {
        let dir = TestDir::new("corrupt");
        std::fs::write(dir.0.join(TEMPLATES_FILE_NAME), "{ not valid json !!!").unwrap();
        let service = TemplatesService::load(&dir.0).unwrap();
        let list = service.list().await;
        assert_eq!(list.len(), BUILTIN_CATALOG.len());
        assert!(list.iter().all(|template| template.builtin));
        // 已重建落盘。
        let raw = std::fs::read_to_string(dir.0.join(TEMPLATES_FILE_NAME)).unwrap();
        assert!(serde_json::from_str::<Vec<AiTemplate>>(&raw).is_ok());
    }

    /// 导入校验接口（NFR-S-07 预留）。
    #[test]
    fn validate_import_checks_structure() {
        // 合法：内置标记被清除、缺 ID 自动生成、孤立模型清空。
        let json = r#"[
            {"id":"correction","name":"纠错模式","prompt":"p1","timeout_ms":5000,"builtin":true},
            {"name":"自定义","prompt":"p2","provider_id":null,"model":"orphan"}
        ]"#;
        let list = validate_import(json).unwrap();
        assert_eq!(list.len(), 2);
        assert!(!list[0].builtin, "导入条目的内置标记一律清除");
        assert!(list[1].id.starts_with("tpl-"), "缺 ID 自动生成");
        assert_eq!(list[1].model, None, "未绑定供应商时孤立模型清空");

        // 非法：JSON 不是数组 / 名称为空 / ID 重复 / 超时超范围。
        assert_eq!(validate_import("{}").unwrap_err().code(), "E-CONFIG-004");
        assert_eq!(
            validate_import(r#"[{"name":"","prompt":"p"}]"#)
                .unwrap_err()
                .code(),
            "E-CONFIG-004"
        );
        assert_eq!(
            validate_import(
                r#"[{"id":"a","name":"x","prompt":"p"},{"id":"a","name":"y","prompt":"p"}]"#
            )
            .unwrap_err()
            .code(),
            "E-CONFIG-004"
        );
        assert_eq!(
            validate_import(r#"[{"name":"x","prompt":"p","timeout_ms":1}]"#)
                .unwrap_err()
                .code(),
            "E-CONFIG-004"
        );
        // 缺省字段按默认（超时 5000）补齐。
        let list = validate_import(r#"[{"name":"x","prompt":"p"}]"#).unwrap();
        assert_eq!(list[0].timeout_ms, DEFAULT_TIMEOUT_MS);
    }
}
