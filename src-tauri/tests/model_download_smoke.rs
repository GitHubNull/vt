//! T-017 模型下载器联网冒烟测试（手动）：真实请求镜像源，验证多线程下载、
//! 断点续传与哈希校验。默认 `#[ignore]`，不参与常规 `cargo test`；
//! 手动执行：`cargo test --test model_download_smoke -- --ignored --nocapture`。

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use vibetype::services::asr::{model_meta, verify_model_hash, MODELS};
use vibetype::services::model_download;

const TOKENS_NAME: &str = "tokens.txt";

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vt-dl-smoke-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("smoke test");
    dir
}

/// 词表文件小（约 75 KB），走单流路径但整体校验全链路（探测 → 下载 → 落盘）。
#[tokio::test]
#[ignore = "联网手动验证：cargo test --test model_download_smoke -- --ignored"]
async fn smoke_download_tokens_from_mirror() {
    let meta = model_meta("paraformer-zh-int8").expect("smoke test");
    let client = model_download::build_client(None).expect("smoke test");
    let dir = temp_dir("tokens");
    let dest = dir.join(TOKENS_NAME);
    let downloaded = Arc::new(AtomicU64::new(0));

    let total = model_download::download_file(
        &client,
        &model_download::mirror_files(meta)[1].url,
        &dest,
        0,
        model_download::DEFAULT_THREADS,
        downloaded,
    )
    .await
    .expect("tokens.txt 下载失败");

    assert!(dest.exists());
    assert_eq!(total, std::fs::metadata(&dest).expect("smoke test").len());
    assert!(total > 0);
}

/// 速度档模型文件（约 223 MB）：多线程分段下载 + 官方 SHA-256 校验。
/// 覆盖"镜像源产物 = 官方登记哈希"的核心验收（T-017）。
#[tokio::test]
#[ignore = "联网手动验证：下载 223 MB，cargo test --test model_download_smoke -- --ignored"]
async fn smoke_download_int8_model_from_mirror_and_verify_hash() {
    let meta = &MODELS[0];
    let client = model_download::build_client(None).expect("smoke test");
    let dir = temp_dir("int8");
    let dest = dir.join("model.onnx");
    let downloaded = Arc::new(AtomicU64::new(0));

    let started = std::time::Instant::now();
    model_download::download_file(
        &client,
        &model_download::mirror_files(meta)[0].url,
        &dest,
        meta.size_bytes,
        model_download::DEFAULT_THREADS,
        downloaded,
    )
    .await
    .expect("模型下载失败");
    let elapsed = started.elapsed();

    let size = std::fs::metadata(&dest).expect("smoke test").len();
    assert_eq!(size, meta.size_bytes, "镜像文件大小与内置元数据不一致");
    // 词表文件同样下载落盘：完整性校验覆盖 model.onnx + tokens.txt（验收 4.6.4）。
    model_download::download_file(
        &client,
        &model_download::mirror_files(meta)[1].url,
        &dir.join(TOKENS_NAME),
        0,
        model_download::DEFAULT_THREADS,
        Arc::new(AtomicU64::new(0)),
    )
    .await
    .expect("tokens.txt 下载失败");
    verify_model_hash(&dir, meta).expect("镜像源 SHA-256 校验失败");
    eprintln!(
        "镜像源下载 {size} 字节耗时 {:?}（平均 {:.1} MB/s）",
        elapsed,
        size as f64 / 1024.0 / 1024.0 / elapsed.as_secs_f64()
    );
}

/// 断点续传：预置一段前缀字节，续传后文件仍与远端一致（单流追加路径）。
#[tokio::test]
#[ignore = "联网手动验证：cargo test --test model_download_smoke -- --ignored"]
async fn smoke_resume_appends_prefix() {
    let meta = model_meta("paraformer-zh-int8").expect("smoke test");
    let client = model_download::build_client(None).expect("smoke test");
    let dir = temp_dir("resume");
    let dest = dir.join(TOKENS_NAME);

    // 整文件下载得基准，再截断前缀模拟中断。
    let full = temp_dir("resume-full").join(TOKENS_NAME);
    model_download::download_file(
        &client,
        &model_download::mirror_files(meta)[1].url,
        &full,
        0,
        1,
        Arc::new(AtomicU64::new(0)),
    )
    .await
    .expect("smoke test");
    let content = std::fs::read(&full).expect("smoke test");
    std::fs::write(&dest, &content[..content.len() / 2]).expect("smoke test");

    model_download::download_file(
        &client,
        &model_download::mirror_files(meta)[1].url,
        &dest,
        0,
        1,
        Arc::new(AtomicU64::new(0)),
    )
    .await
    .expect("smoke test");
    assert_eq!(
        std::fs::read(&dest).expect("smoke test"),
        content,
        "续传后内容与整文件不一致"
    );
}
