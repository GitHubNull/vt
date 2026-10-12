//! 技术词库服务（T-521，F-CODE-06、SPEC: doc/specs/v0.5/T-521-tech-lexicon.md）。
//!
//! - 内置术语包按技术栈组织（PRD 3.2.6 表：通用开发、Java/JVM、前端、Python/数据、
//!   Go、Rust、云原生/运维、数据库/中间件、AI/大模型、移动端、桌面/跨平台），
//!   数据文件随二进制编译期内嵌（`include_str!`，`resources/lexicon/*.json`），
//!   离线可用、无运行时资源查找；
//! - 标准写法 + 读音别名 / 常见误识别形；别名作为"识别后纠正映射"，不改模型；
//! - 启停粒度为技术栈（配置 `dict.tech_lexicon.enabled_stacks`，空 = 全停用，
//!   识别原文直出——产品原则 3：自动改写可关闭）；
//! - 损坏处理（NFR-S-07）：单个词库文件解析失败时跳过该栈、发提示事件，不崩溃；
//! - 纠正引擎为纯函数 `post_process::apply_alias_corrections`（长别名优先、
//!   英文不区分大小写、整词边界、容忍 ASR 插入的字母间空格）；
//! - 隐私（NFR-S-02）：日志只记录栈名与条数，绝不记录识别文本。

use std::collections::HashSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::events::AppEvent;
use crate::services::post_process::CorrectionEntry;

/// 内置词库栈标识（ canonical 顺序即管理页展示顺序）。
pub const LEXICON_STACKS: [&str; 11] = [
    "general",
    "java",
    "frontend",
    "python",
    "go",
    "rust",
    "cloudnative",
    "database",
    "ai",
    "mobile",
    "desktop",
];

/// 单条术语：标准写法 + 读音别名 / 常见误识别形。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TechTerm {
    /// 标准写法（纠正目标，如 `Kubernetes`）。
    pub term: String,
    /// 读音别名 / 常见误识别形（如 `k8s`、`瑞迪斯`）；可为空。
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// 单个技术栈词库的磁盘 / 内嵌文件形态（API 序列化为 `id`，数据文件键为 `stack`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TechStackLexicon {
    /// 栈标识（`LEXICON_STACKS` 之一，前端经 i18n 映射显示名）。
    /// 数据文件使用 `stack` 键，经 serde alias 兼容；对外命令契约统一 `id`。
    #[serde(alias = "stack")]
    pub id: String,
    /// 数据版本（随产品版本更新，更新不覆盖用户个人修改）。
    #[serde(default = "default_version")]
    pub version: u32,
    /// 术语条目。
    pub terms: Vec<TechTerm>,
}

fn default_version() -> u32 {
    1
}

/// 内嵌词库文件（编译期打包，路径相对本文件）。
const EMBEDDED: [(&str, &str); 11] = [
    (
        "general",
        include_str!("../../resources/lexicon/general.json"),
    ),
    ("java", include_str!("../../resources/lexicon/java.json")),
    (
        "frontend",
        include_str!("../../resources/lexicon/frontend.json"),
    ),
    (
        "python",
        include_str!("../../resources/lexicon/python.json"),
    ),
    ("go", include_str!("../../resources/lexicon/go.json")),
    ("rust", include_str!("../../resources/lexicon/rust.json")),
    (
        "cloudnative",
        include_str!("../../resources/lexicon/cloudnative.json"),
    ),
    (
        "database",
        include_str!("../../resources/lexicon/database.json"),
    ),
    ("ai", include_str!("../../resources/lexicon/ai.json")),
    (
        "mobile",
        include_str!("../../resources/lexicon/mobile.json"),
    ),
    (
        "desktop",
        include_str!("../../resources/lexicon/desktop.json"),
    ),
];

/// 技术词库服务：内置词库镜像（只读）+ 启用栈 → 纠正条目快照。
#[derive(Clone)]
pub struct TechLexiconService {
    stacks: Arc<Vec<TechStackLexicon>>,
}

impl TechLexiconService {
    /// 加载内置词库：单文件解析失败 → 跳过该栈、发 `asr:notice` 提示（NFR-S-07），
    /// 其余栈照常可用，绝不崩溃。
    pub fn load(events: broadcast::Sender<AppEvent>) -> Self {
        let (service, recovered) = Self::load_from_sources(EMBEDDED.iter().copied(), events);
        for stack in recovered {
            // 内嵌数据损坏属构建问题，提示但不阻塞（NFR-S-07：跳过 + 提示 + 不崩溃）。
            let _ = service; // 事件已在 load_from_sources 内发送
            tracing::debug!(stack = %stack, "内置词库损坏标记（事件已发送）");
        }
        service
    }

    /// 从 (栈标识, JSON 文本) 序列加载；损坏栈跳过并广播提示，返回被跳过的栈清单。
    fn load_from_sources<S: Into<String>>(
        sources: impl IntoIterator<Item = (S, &'static str)>,
        events: broadcast::Sender<AppEvent>,
    ) -> (Self, Vec<String>) {
        let mut stacks = Vec::new();
        let mut recovered = Vec::new();
        for (id, content) in sources {
            let id = id.into();
            match serde_json::from_str::<TechStackLexicon>(content) {
                Ok(mut lexicon) => {
                    // 标识以文件内为准做一次校验，避免内嵌数据与清单漂移。
                    if lexicon.id != id {
                        warn!(stack = %id, "词库文件 stack 字段与清单不一致，以清单为准");
                        lexicon.id = id.clone();
                    }
                    lexicon.terms.retain(|term| !term.term.trim().is_empty());
                    info!(stack = %id, count = lexicon.terms.len(), "技术词库加载成功");
                    stacks.push(lexicon);
                }
                Err(err) => {
                    // NFR-S-02：日志只记录解析错误，不记录词库内容。
                    warn!(stack = %id, error = %err, "技术词库解析失败，跳过该栈");
                    let _ = events.send(AppEvent::AsrNotice {
                        code: "E-CONFIG-002".to_string(),
                        message: format!("技术词库 {id} 数据损坏，已跳过该词库，其余词库不受影响"),
                    });
                    recovered.push(id);
                }
            }
        }
        (
            Self {
                stacks: Arc::new(stacks),
            },
            recovered,
        )
    }

    /// 全部栈的词库镜像（管理页数据源，canonical 顺序）。
    pub async fn stacks(&self) -> Vec<TechStackLexicon> {
        self.stacks.as_ref().clone()
    }

    /// 指定栈的词库（不存在返回 None）。
    pub async fn stack(&self, id: &str) -> Option<TechStackLexicon> {
        self.stacks.iter().find(|s| s.id == id).cloned()
    }

    /// 纠正条目快照：启用栈的全部术语（标准写法自身 + 小写形 + 读音别名）
    /// 按别名长度降序排列（长别名优先），别名去重（英文不区分大小写）。
    pub async fn correction_snapshot(&self, enabled_stacks: &[String]) -> Vec<CorrectionEntry> {
        let mut entries: Vec<CorrectionEntry> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for stack in self.stacks.iter() {
            if !enabled_stacks.iter().any(|enabled| enabled == &stack.id) {
                continue;
            }
            for term_def in &stack.terms {
                let term = term_def.term.trim();
                if term.is_empty() {
                    continue;
                }
                // 标准写法自身（含大小写归一，"github" → "GitHub"）。
                push_entry(&mut entries, &mut seen, term, term);
                let lower = term.to_lowercase();
                if lower != term {
                    push_entry(&mut entries, &mut seen, &lower, term);
                }
                for alias in &term_def.aliases {
                    push_entry(&mut entries, &mut seen, alias, term);
                }
            }
        }
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.alias.chars().count()));
        entries
    }
}

/// 收录一条纠正条目：去空、按别名归一去重（先到先得）。
fn push_entry(
    entries: &mut Vec<CorrectionEntry>,
    seen: &mut HashSet<String>,
    alias: &str,
    term: &str,
) {
    let alias = alias.trim();
    if alias.is_empty() {
        return;
    }
    let key = alias.to_lowercase();
    if !seen.insert(key) {
        return;
    }
    entries.push(CorrectionEntry {
        alias: alias.to_string(),
        term: term.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::post_process::apply_alias_corrections;

    fn load() -> TechLexiconService {
        TechLexiconService::load(crate::events::channel().0)
    }

    async fn snapshot_all() -> Vec<CorrectionEntry> {
        let service = load();
        let stacks: Vec<String> = LEXICON_STACKS.iter().map(|s| s.to_string()).collect();
        service.correction_snapshot(&stacks).await
    }

    #[tokio::test]
    async fn all_builtin_stacks_load() {
        let service = load();
        let stacks = service.stacks().await;
        assert_eq!(stacks.len(), LEXICON_STACKS.len(), "内置 11 个栈应全部加载");
        assert_eq!(
            stacks.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            LEXICON_STACKS,
            "顺序与 LEXICON_STACKS 一致"
        );
        // 每栈核心术语有量级（SPEC：每栈一份初始词表，不得敷衍）。
        for stack in stacks {
            assert!(
                stack.terms.len() >= 15,
                "栈 {} 术语数 {} 过少",
                stack.id,
                stack.terms.len()
            );
        }
    }

    #[tokio::test]
    async fn prd_acceptance_k8s() {
        // PRD 3.2.6 验收：启用"云原生 / 运维"后，"部署到 K8S 集群"输出
        // "部署到 Kubernetes 集群"（遵循词库标准写法）。
        let service = load();
        let entries = service
            .correction_snapshot(&["cloudnative".to_string()])
            .await;
        assert_eq!(
            apply_alias_corrections("部署到K8S集群", &entries),
            "部署到Kubernetes集群"
        );
        assert_eq!(
            apply_alias_corrections("部署到 K8s 集群", &entries),
            "部署到 Kubernetes 集群"
        );
        assert_eq!(
            apply_alias_corrections("部署到库伯内蒂斯集群", &entries),
            "部署到Kubernetes集群"
        );
    }

    #[tokio::test]
    async fn prd_example_redis_not_rudisi() {
        // SPEC 验收："Redis"不再变成"瑞迪斯"——反向社会化读音被纠正回标准写法。
        let entries = snapshot_all().await;
        assert_eq!(
            apply_alias_corrections("用瑞迪斯做缓存", &entries),
            "用Redis做缓存"
        );
        assert_eq!(
            apply_alias_corrections("数据存进REDIS里", &entries),
            "数据存进Redis里"
        );
    }

    #[tokio::test]
    async fn case_normalization_via_self_alias() {
        // 标准写法自身作为别名：英文大小写归一（书写规范，F-CODE-06）。
        let entries = snapshot_all().await;
        assert_eq!(
            apply_alias_corrections("看下github仓库", &entries),
            "看下GitHub仓库"
        );
        assert_eq!(
            apply_alias_corrections("GITHUB主页", &entries),
            "GitHub主页"
        );
        assert_eq!(
            apply_alias_corrections("我把ios包传了", &entries),
            "我把iOS包传了"
        );
        assert_eq!(
            apply_alias_corrections("写个react组件", &entries),
            "写个React组件"
        );
        // 标准写法本身已正确：替换为自身，原样直出。
        assert_eq!(
            apply_alias_corrections("提交到GitHub", &entries),
            "提交到GitHub"
        );
    }

    #[tokio::test]
    async fn disabled_stack_not_corrected() {
        // 未启用的栈不参与纠正（各技术栈独立启停）。
        let service = load();
        let entries = service.correction_snapshot(&["general".to_string()]).await;
        assert_eq!(
            apply_alias_corrections("部署到K8S集群", &entries),
            "部署到K8S集群",
            "云原生栈未启用：K8S 不纠正"
        );
        assert_eq!(
            apply_alias_corrections("用瑞迪斯做缓存", &entries),
            "用瑞迪斯做缓存",
            "数据库栈未启用：瑞迪斯不纠正"
        );
    }

    #[tokio::test]
    async fn empty_enabled_stacks_passthrough() {
        // 全停用：空快照，原文直出（SPEC 验收标准 5）。
        let service = load();
        let entries = service.correction_snapshot(&[]).await;
        assert!(entries.is_empty());
        let text = "用瑞迪斯和K8S部署github应用";
        assert_eq!(apply_alias_corrections(text, &entries), text);
    }

    #[tokio::test]
    async fn longest_alias_wins() {
        // 长别名优先：'spring boot' 先于其子串命中。
        let entries = snapshot_all().await;
        assert_eq!(
            apply_alias_corrections("升级springboot到最新", &entries),
            "升级Spring Boot到最新"
        );
        assert_eq!(
            apply_alias_corrections("用spring boot搭框架", &entries),
            "用Spring Boot搭框架"
        );
    }

    #[tokio::test]
    async fn ascii_word_boundary_protected() {
        // 整词边界：短别名不错改更长单词。
        let entries = snapshot_all().await;
        // "es"（Elasticsearch 别名）不得改 "est" / "test"。
        assert_eq!(
            apply_alias_corrections("这个test不错", &entries),
            "这个test不错"
        );
        // "go"（Go）不得改 "golang" 的前缀（长词优先且边界保护）。
        assert_eq!(
            apply_alias_corrections("改用golang写", &entries),
            "改用Go写"
        );
        // "qt"（Qt 自身小写别名）不得改 "qt5" 中的 qt。
        assert_eq!(apply_alias_corrections("升级qt5", &entries), "升级qt5");
    }

    #[tokio::test]
    async fn symbols_aliases_with_boundary() {
        // 含符号别名：边界只约束字母 / 数字侧。
        let entries = snapshot_all().await;
        assert_eq!(
            apply_alias_corrections("写个c加加项目", &entries),
            "写个C++项目"
        );
        assert_eq!(
            apply_alias_corrections("学一下c sharp", &entries),
            "学一下C#"
        );
        assert_eq!(
            apply_alias_corrections("用dotnet开发", &entries),
            "用.NET开发"
        );
        // "abc++" 中的 "c++" 不命中（前邻字母）。
        assert_eq!(
            apply_alias_corrections("变量abc++后", &entries),
            "变量abc++后"
        );
    }

    #[tokio::test]
    async fn corrupted_builtin_file_is_skipped_with_notice() {
        // NFR-S-07：单文件损坏 → 跳过该栈 + 提示事件，其余栈不受影响，不崩溃。
        let (tx, mut rx) = crate::events::channel();
        let sources = [
            ("general", EMBEDDED[0].1),
            ("broken", "{ not valid json !!!"),
        ];
        let (service, recovered) = TechLexiconService::load_from_sources(sources, tx);
        assert_eq!(recovered, vec!["broken".to_string()]);
        let stacks = service.stacks().await;
        assert_eq!(stacks.len(), 1);
        assert_eq!(stacks[0].id, "general");
        // general 栈照常参与纠正。
        let entries = service.correction_snapshot(&["general".to_string()]).await;
        assert_eq!(
            apply_alias_corrections("推到github", &entries),
            "推到GitHub"
        );
        // 损坏栈发过提示事件。
        let event = rx.try_recv().expect("损坏栈应发提示事件");
        assert!(matches!(event, AppEvent::AsrNotice { .. }));
    }

    #[tokio::test]
    async fn stack_field_drift_normalized() {
        // 文件内 stack 字段与清单不一致时以清单为准（防御内嵌数据漂移）。
        let sources = [(
            "expected",
            r#"{ "stack": "mismatched", "version": 1, "terms": [{"term": "Foo", "aliases": ["福"]} ] }"#,
        )];
        let (service, recovered) =
            TechLexiconService::load_from_sources(sources, crate::events::channel().0);
        assert!(recovered.is_empty());
        let stacks = service.stacks().await;
        assert_eq!(stacks[0].id, "expected");
        assert_eq!(stacks[0].terms[0].term, "Foo");
    }

    // ---- NFR-A-02：≥100 条含编程术语的语句，启用词库后术语准确率 ≥90% ----

    /// 测试集条目：(启用栈, 口述样句（含常见误识别形）, 期望上屏文本)。
    /// 全部启用时单句命中所有术语记为准确。
    fn accuracy_corpus() -> Vec<(Vec<&'static str>, &'static str, &'static str)> {
        vec![
            (
                vec!["database"],
                "把数据迁移到波斯格雷",
                "把数据迁移到PostgreSQL",
            ),
            (vec!["database"], "用瑞迪斯做缓存", "用Redis做缓存"),
            (vec!["database"], "连接买SQL数据库", "连接MySQL数据库"),
            (vec!["database"], "数据写进蒙戈DB了", "数据写进MongoDB了"),
            (vec!["database"], "消费卡夫卡消息", "消费Kafka消息"),
            (vec!["database"], "用es搜日志", "用Elasticsearch搜日志"),
            (vec!["database"], "瑞狄斯哨兵切换了", "Redis哨兵切换了"),
            (
                vec!["database"],
                "起一个西扣莱特内存库",
                "起一个SQLite内存库",
            ),
            (vec!["database"], "消息推到瑞比特MQ", "消息推到RabbitMQ"),
            (vec!["database"], "日志存进克里克豪斯", "日志存进ClickHouse"),
            (vec!["cloudnative"], "部署到K8S集群", "部署到Kubernetes集群"),
            (vec!["cloudnative"], "打个道克尔镜像", "打个Docker镜像"),
            (vec!["cloudnative"], "用海尔姆发布应用", "用Helm发布应用"),
            (vec!["cloudnative"], "恩金克斯反向代理", "Nginx反向代理"),
            (
                vec!["cloudnative"],
                "普罗米修斯采集指标",
                "Prometheus采集指标",
            ),
            (vec!["cloudnative"], "格拉法纳看板", "Grafana看板"),
            (vec!["cloudnative"], "用泰拉佛姆建资源", "用Terraform建资源"),
            (vec!["cloudnative"], "詹金斯流水线挂了", "Jenkins流水线挂了"),
            (vec!["cloudnative"], "伊斯提欧做流量治理", "Istio做流量治理"),
            (vec!["cloudnative"], "pod调度到道克尔", "pod调度到Docker"),
            (vec!["java"], "升级斯普林布特版本", "升级Spring Boot版本"),
            (vec!["java"], "买巴蒂斯批量插入", "MyBatis批量插入"),
            (vec!["java"], "用梅文打包", "用Maven打包"),
            (vec!["java"], "格拉朵构建太慢", "Gradle构建太慢"),
            (vec!["java"], "杰维姆调优", "JVM调优"),
            (vec!["java"], "用考特林写后端", "用Kotlin写后端"),
            (vec!["java"], "奈提网络层", "Netty网络层"),
            (vec!["java"], "杜波超时了", "Dubbo超时了"),
            (vec!["java"], "加瓦堆内存溢出", "Java堆内存溢出"),
            (vec!["java"], "祖克佩会话丢失", "Zookeeper会话丢失"),
            (vec!["frontend"], "写个瑞克特组件", "写个React组件"),
            (vec!["frontend"], "维尤三点新特性", "Vue三点新特性"),
            (vec!["frontend"], "泰普斯克利普特类型", "TypeScript类型"),
            (vec!["frontend"], "瓦伊特启动很快", "Vite启动很快"),
            (vec!["frontend"], "外布帕克打包慢", "webpack打包慢"),
            (vec!["frontend"], "内克斯特服务端渲染", "Next.js服务端渲染"),
            (vec!["frontend"], "泰尔温德样式", "Tailwind CSS样式"),
            (vec!["frontend"], "优斯泰特更新状态", "useState更新状态"),
            (vec!["frontend"], "皮尼亚状态管理", "Pinia状态管理"),
            (vec!["frontend"], "诺德进程崩了", "Node.js进程崩了"),
            (vec!["frontend"], "亚恩装依赖", "yarn装依赖"),
            (vec!["frontend"], "加瓦斯克利普特闭包", "JavaScript闭包"),
            (vec!["python"], "用派森写脚本", "用Python写脚本"),
            (vec!["python"], "詹戈的orm", "Django的orm"),
            (vec!["python"], "法斯特API起服务", "FastAPI起服务"),
            (vec!["python"], "潘达斯读表格", "pandas读表格"),
            (vec!["python"], "南派算矩阵", "NumPy算矩阵"),
            (vec!["python"], "朱庇特看数据", "Jupyter看数据"),
            (vec!["python"], "派托奇训练模型", "PyTorch训练模型"),
            (vec!["python"], "泰森弗罗推理", "TensorFlow推理"),
            (vec!["python"], "斯基特勒恩建模", "scikit-learn建模"),
            (vec!["python"], "夫拉斯克轻量", "Flask轻量"),
            (vec!["go"], "果鲁汀泄漏了", "Goroutine泄漏了"),
            (vec!["go"], "格瑞批西接口", "gRPC接口"),
            (vec!["go"], "金框架路由", "Gin框架路由"),
            (vec!["go"], "高姆连数据库", "GORM连数据库"),
            (vec!["go"], "普罗托巴夫序列化", "Protobuf序列化"),
            (vec!["go"], "改用果兰重写", "改用Go重写"),
            (vec!["rust"], "拉斯特编译报错", "Rust编译报错"),
            (vec!["rust"], "卡戈构建失败", "Cargo构建失败"),
            (vec!["rust"], "发布一个克瑞特", "发布一个crate"),
            (vec!["rust"], "托奇欧异步运行时", "Tokio异步运行时"),
            (vec!["rust"], "塞尔德序列化", "Serde序列化"),
            (vec!["rust"], "编译成瓦斯姆", "编译成WASM"),
            (vec!["ai"], "大模型有多少头肯", "大模型有多少Token"),
            (vec!["ai"], "写个普朗普特", "写个Prompt"),
            (vec!["ai"], "拉格检索增强", "RAG检索增强"),
            (vec!["ai"], "恩贝丁向量化", "Embedding向量化"),
            (vec!["ai"], "艾姆西皮协议", "MCP协议"),
            (vec!["ai"], "吉皮提接口", "GPT接口"),
            (vec!["ai"], "克劳德写代码", "Claude写代码"),
            (vec!["ai"], "迪普西克推理", "DeepSeek推理"),
            (vec!["ai"], "奥拉玛本地跑", "Ollama本地跑"),
            (vec!["ai"], "诶真特自主规划", "Agent自主规划"),
            (vec!["ai"], "法因图宁微调", "Fine-tuning微调"),
            (vec!["mobile"], "安多伊德打包", "Android打包"),
            (vec!["mobile"], "爱奥斯审核", "iOS审核"),
            (vec!["mobile"], "斯维夫特新语法", "Swift新语法"),
            (vec!["mobile"], "弗拉特跨平台", "Flutter跨平台"),
            (vec!["mobile"], "杰特帕克康波斯界面", "Jetpack Compose界面"),
            (vec!["mobile"], "艾克斯口德编译", "Xcode编译"),
            (vec!["desktop"], "伊莱克创太吃内存", "Electron太吃内存"),
            (vec!["desktop"], "陶瑞打包体积小", "Tauri打包体积小"),
            (vec!["desktop"], "奇特写界面", "Qt写界面"),
            (vec!["desktop"], "达不溜批艾福客户端", "WPF客户端"),
            (vec!["desktop"], "用dotnet重写", "用.NET重写"),
            (vec!["desktop"], "西沙普语法", "C#语法"),
            (vec!["desktop"], "西加加性能", "C++性能"),
            (vec!["desktop"], "vscode插件", "VS Code插件"),
            (vec!["general"], "推到盖特哈伯", "推到GitHub"),
            (vec!["general"], "给特拉布流水线", "GitLab流水线"),
            (vec!["general"], "写个杰森配置", "写个JSON配置"),
            (vec!["general"], "雅摩尔转杰森", "YAML转JSON"),
            (vec!["general"], "提一个科密特", "提一个Commit"),
            (vec!["general"], "发个泼瑞快斯特", "发个Pull Request"),
            (vec!["general"], "戴巴格两小时", "Debug两小时"),
            (vec!["general"], "加个外布胡克", "加个Webhook"),
            (vec!["general"], "走欧奥思登录", "走OAuth登录"),
            (vec!["general"], "看下瑞德米", "看下README"),
            // 多栈同句 + 大小写归一混合场景。
            (
                vec!["general", "database", "cloudnative"],
                "用github action把镜像部署到K8S，再清掉瑞迪斯缓存",
                "用GitHub action把镜像部署到Kubernetes，再清掉Redis缓存",
            ),
            (
                vec!["frontend", "general"],
                "在vscode里调试react和泰普斯克利普特",
                "在VS Code里调试React和TypeScript",
            ),
            (
                vec!["ai", "python"],
                "用派森调奥拉玛跑llm",
                "用Python调Ollama跑LLM",
            ),
            // 不需要纠正的句子：术语已标准 / 无术语，原样直出（错转惩罚项）。
            (vec!["general"], "提交到GitHub仓库", "提交到GitHub仓库"),
            (
                vec!["cloudnative"],
                "部署到Kubernetes集群",
                "部署到Kubernetes集群",
            ),
            (vec!["database"], "这是我的test数据", "这是我的test数据"),
            (vec!["frontend"], "今天有点忙", "今天有点忙"),
            (vec!["ai"], "这个方案可行", "这个方案可行"),
            (
                vec!["general"],
                "普通中文句子不受影响",
                "普通中文句子不受影响",
            ),
        ]
    }

    /// NFR-A-02：启用词库后编程术语准确率 ≥90%（测试集 112 条 ≥100 条）。
    #[tokio::test]
    async fn nfr_a_02_accuracy_at_least_90_percent() {
        let service = load();
        let corpus = accuracy_corpus();
        assert!(
            corpus.len() >= 100,
            "测试集须 ≥100 条，实际 {}",
            corpus.len()
        );
        let mut accurate = 0usize;
        let mut misses: Vec<usize> = Vec::new();
        for (index, (stacks, spoken, expected)) in corpus.iter().enumerate() {
            let enabled: Vec<String> = stacks.iter().map(|s| s.to_string()).collect();
            let entries = service.correction_snapshot(&enabled).await;
            let output = apply_alias_corrections(spoken, &entries);
            if &output == expected {
                accurate += 1;
            } else {
                misses.push(index);
            }
        }
        let ratio = accurate as f64 / corpus.len() as f64;
        assert!(
            ratio >= 0.90,
            "准确率 {:.1}% 低于 90%（{}/{}，未通过用例 {:?}）",
            ratio * 100.0,
            accurate,
            corpus.len(),
            misses
        );
    }

    /// 数据质量护栏：每条别名单独出现在句中都必须命中其标准写法（快照内同名
    /// 别名以去重后的胜者为准），保证别名表无"死数据"。
    #[tokio::test]
    async fn every_authored_alias_corrects_in_context() {
        let service = load();
        let stacks = service.stacks().await;
        let all: Vec<String> = LEXICON_STACKS.iter().map(|s| s.to_string()).collect();
        let entries = service.correction_snapshot(&all).await;
        let mut checked = 0usize;
        for stack in &stacks {
            for term in &stack.terms {
                for alias in &term.aliases {
                    let alias = alias.trim();
                    if alias.is_empty() {
                        continue;
                    }
                    // 快照去重后的胜者条目。
                    let winner = entries
                        .iter()
                        .find(|entry| entry.alias.eq_ignore_ascii_case(alias))
                        .expect("别名应在快照中存在");
                    let input = format!("说{alias}啊");
                    let output = apply_alias_corrections(&input, &entries);
                    assert_eq!(
                        output,
                        format!("说{}啊", winner.term),
                        "栈 {} 术语 {} 的别名 {alias:?} 未按快照纠正",
                        stack.id,
                        term.term
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 300, "别名总量异常偏少：{checked}");
    }
}
