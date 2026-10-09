//! T-006 集成测试：固定测试音频（sherpa-onnx 官方 test_wavs）→ 识别结果包含预期关键词。
//!
//! 模型来源：仓库 `tmp/models-cache/`（预置缓存，1 GB 归档已完整校验；
//! tmp/ 已 gitignore，CI 环境可设 `VT_MODELS_CACHE` 指向等价目录，缺省时跳过）。
//! 集成测试加载 int8 档位模型（速度优先，内存占用小），验证完整
//! “加载 → 推理 → 文本”链路；fp32 档位 accuracy 基准需基准机实测（见 SPEC 验收）。
//!
//! 测试 staging 代码允许 unwrap（与 src/ 内单测一致；Cargo.toml 对 src/ 的
//! unwrap_used 告警不覆盖 cfg(test) 之外的本文件）。
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use vibetype::services::asr::{AsrService, ModelTier};
use vibetype::services::config_store::ConfigStore;

/// 预置模型缓存根目录（tmp/models-cache，gitignore 不入库）。
fn models_cache_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("VT_MODELS_CACHE") {
        return Some(PathBuf::from(dir));
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cache = manifest.join("../tmp/models-cache");
    cache.is_dir().then_some(cache)
}

/// 把缓存中的模型文件硬链接（同盘零拷贝）到临时 models/<id>/ 目录。
fn stage_model(models_dir: &Path, id: &str, cache: &Path) {
    let extracted = cache.join("extracted/sherpa-onnx-paraformer-zh-2023-03-28");
    let target = models_dir.join(id);
    std::fs::create_dir_all(&target).unwrap();
    let model_name = if id == "paraformer-zh-int8" {
        "model.int8.onnx"
    } else {
        "model.onnx"
    };
    std::fs::hard_link(extracted.join(model_name), target.join("model.onnx"))
        .or_else(|_| {
            std::fs::copy(extracted.join(model_name), target.join("model.onnx")).map(|_| ())
        })
        .unwrap();
    std::fs::hard_link(extracted.join("tokens.txt"), target.join("tokens.txt"))
        .or_else(|_| {
            std::fs::copy(extracted.join("tokens.txt"), target.join("tokens.txt")).map(|_| ())
        })
        .unwrap();
}

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vt-asr-it-{name}-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn recognize_test_wavs_contains_expected_keywords() {
    let Some(cache) = models_cache_dir() else {
        eprintln!("跳过：未找到模型缓存（tmp/models-cache 或 VT_MODELS_CACHE）");
        return;
    };
    let dir = test_dir("recognize");
    let models_dir = dir.join("models");
    std::fs::create_dir_all(&models_dir).unwrap();
    stage_model(&models_dir, "paraformer-zh-int8", &cache);

    let (events_tx, _rx) = vibetype::events::channel();
    let config = ConfigStore::load(&dir, events_tx.clone()).unwrap();
    let service = Arc::new(AsrService::new(models_dir, config, events_tx));
    service.reload_engine().await.expect("引擎加载失败");

    // 官方测试音频与预期关键词（模型发布包 test_wavs，普通话通用文本）。
    let wav_dir = cache.join("extracted/sherpa-onnx-paraformer-zh-2023-03-28/test_wavs");
    // 关键词来自 int8 模型对官方 test_wavs 的实际识别输出（本地实测，见 SPEC 验收说明）。
    let cases = [
        ("0.wav", &["感兴趣"][..]),
        ("1.wav", &["金融动荡"][..]),
        ("2.wav", &["根源"][..]),
    ];
    for (wav, keywords) in cases {
        let path = wav_dir.join(wav);
        if !path.exists() {
            eprintln!("跳过 {wav}：测试音频缺失");
            continue;
        }
        let text = service
            .recognize_wav(path.to_string_lossy().into_owned())
            .await
            .unwrap_or_else(|err| panic!("{wav} 识别失败: {}", err.code()));
        assert!(
            keywords.iter().any(|kw| text.contains(kw)),
            "{wav} 识别结果应包含关键词 {keywords:?}，实际: {text}"
        );
        eprintln!("{wav} → {text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn model_registry_exposes_two_tiers() {
    use vibetype::services::asr::MODELS;
    assert_eq!(MODELS.len(), 2);
    let tiers: Vec<ModelTier> = MODELS.iter().map(|meta| meta.tier).collect();
    assert!(tiers.contains(&ModelTier::Speed));
    assert!(tiers.contains(&ModelTier::Accuracy));
}
