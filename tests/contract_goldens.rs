//! 前后端契约金样对拍。
//!
//! 把关键契约类型的真实序列化样例写入
//! `packages/shared/contracts/rust-contracts.json`；
//! TS 侧 `scripts/architecture/contract-parity.test.ts` 用 zod schema 对拍
//! （解析成功 + 字段集双向校验）。任何一侧漂移都会让测试变红。
//!
//! 契约有意的变更流程：
//!   1. 改 Rust 类型
//!   2. UPDATE_CONTRACT_GOLDENS=1 cargo test -p MineRadio-api --test contract_goldens
//!   3. 按新金样同步 packages/shared 的 zod schema 与消费方

use std::{fs, path::Path};

use mineradio_api::types::{
    PlayableState, PlaylistDetail, PreviewRange, ProviderId, RecommendationCard,
    RecommendationCardKind, RecommendationModule, RecommendationModuleKind, RecommendationPage,
    SongUrlResult, Track,
};

fn sample_track() -> Track {
    Track {
        id: "t-1".to_owned(),
        provider: ProviderId::Netease,
        source_id: "t-1".to_owned(),
        media_mid: None,
        title: "示例歌曲".to_owned(),
        artists: vec!["示例歌手".to_owned()],
        album: "示例专辑".to_owned(),
        cover_url: "https://example.com/cover.jpg".to_owned(),
        quality_hints: vec![],
        playable_state: PlayableState::Playable,
        duration_ms: Some(180_000),
    }
}

fn sample_song_url_result() -> SongUrlResult {
    SongUrlResult {
        url: "https://example.com/audio.flac".to_owned(),
        quality: "lossless".to_owned(),
        expires_at: Some("2026-08-26T12:00:00Z".to_owned()),
        preview_range: Some(PreviewRange {
            start_ms: 0,
            end_ms: 30_000,
        }),
    }
}

fn sample_playlist_detail() -> PlaylistDetail {
    PlaylistDetail {
        provider: ProviderId::Qq,
        id: "pl-1".to_owned(),
        name: "示例歌单".to_owned(),
        cover_url: "https://example.com/pl-cover.jpg".to_owned(),
        track_count: Some(1),
        track_ids: vec!["t-1".to_owned()],
        collected: Some(false),
        tracks: vec![sample_track()],
        has_more: Some(true),
    }
}

fn sample_recommendation_page() -> RecommendationPage {
    RecommendationPage {
        provider: ProviderId::Netease,
        list: vec![RecommendationModule {
            title: "每日推荐".to_owned(),
            kind: RecommendationModuleKind::Mixed,
            list: vec![RecommendationCard {
                id: "card-1".to_owned(),
                title: "示例歌名".to_owned(),
                subtitle: "示例歌手".to_owned(),
                cover_url: "https://example.com/card.jpg".to_owned(),
                collected: Some(false),
                kind: RecommendationCardKind::Track,
            }],
        }],
    }
}

#[test]
fn contract_goldens_match_shared_snapshots() {
    // 只有显式设置环境变量时才执行，否则跳过, 避免路径穿越。
    if std::env::var_os("MINERADIO_CONTRACT_GOLDENS").is_none() {
        eprintln!(
            "skip contract goldens：设置 MINERADIO_CONTRACT_GOLDENS=1 后执行（根仓库的 contracts 脚本会自动带上）"
        );
        return;
    }
    let mut root = serde_json::Map::new();
    root.insert(
        "SongUrlResult".to_owned(),
        serde_json::to_value(sample_song_url_result()).expect("serialize SongUrlResult"),
    );
    root.insert(
        "Track".to_owned(),
        serde_json::to_value(sample_track()).expect("serialize Track"),
    );
    root.insert(
        "PlaylistDetail".to_owned(),
        serde_json::to_value(sample_playlist_detail()).expect("serialize PlaylistDetail"),
    );
    root.insert(
        "RecommendationPage".to_owned(),
        serde_json::to_value(sample_recommendation_page()).expect("serialize RecommendationPage"),
    );
    let rendered = format!(
        "{}\n",
        serde_json::to_string_pretty(&serde_json::Value::Object(root)).expect("render goldens")
    );

    let golden_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../packages/shared/contracts/rust-contracts.json");

    if std::env::var_os("UPDATE_CONTRACT_GOLDENS").is_some() {
        if let Some(parent) = golden_path.parent() {
            fs::create_dir_all(parent)
                .unwrap_or_else(|err| panic!("创建契约目录失败 {:?}: {err}", parent));
        }
        fs::write(&golden_path, rendered)
            .unwrap_or_else(|err| panic!("写入契约金样失败 {:?}: {err}", golden_path));
        return;
    }

    let committed = fs::read_to_string(&golden_path).unwrap_or_else(|_| {
        panic!(
            "契约金样缺失：运行 UPDATE_CONTRACT_GOLDENS=1 cargo test -p MineRadio-api --test contract_goldens 生成"
        )
    });
    assert_eq!(
        committed.replace("\r\n", "\n"),
        rendered,
        "Rust 契约与前端金样漂移：确认变更有意后运行 \
         UPDATE_CONTRACT_GOLDENS=1 cargo test -p MineRadio-api --test contract_goldens \
         更新金样，并同步 packages/shared 的 zod schema 与消费方"
    );
}
