use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    emit_librespot_env();
    compile_librespot_protocol();
    deploy_csigner_resources();
}

fn deploy_csigner_resources() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));

    let target = env::var("TARGET").expect("TARGET");

    println!("cargo:warning=csigner: 目标平台 {target}");

    //mac
    let arch = if target.starts_with("x86_64-") {
        "x86_64"
    } else if target.starts_with("i686-") {
        "x86"
    } else if target.starts_with("aarch64-") {
        "arm64"
    } else {
        println!("cargo:warning=csigner: 不支持的架构 {target}");
        return;
    };

    //                     x86        x86_64        arm64
    //Windows             .dll/.bin   .dll/.bin     .dll
    //Linux               .so/.bin    .so/.bin      .so
    //macOS               .dylib      .dylib/.bin   .dylib/.bin
    //bin是附加签名包, 动态链接库包含2个独立签名函数 以及附加包调用签名
    
    let (ext, bin) = if target.contains("windows") {
        ("dll", "wine")
    } else if target.contains("linux") {
        ("so", "wine")
    } else if target.contains("apple") {
        ("dylib", "macos")
    } else {
        println!("cargo:warning=csigner: 不支持的操作系统 {target}");
        return;
    };

    let filename = format!("{arch}.{ext}");
    let bin_filename = format!("{bin}-{arch}.bin");

    let resources_dir = manifest_dir.join("resources");
    let source = resources_dir.join(&filename);

    if !source.is_file() {
        println!("cargo:warning=csigner: 未找到动态库 {}", source.display());
        return;
    }

    // 业务方输出目录
    let out_dir = env::var("OUT_DIR")
        .ok()
        .and_then(|out| PathBuf::from(out).ancestors().nth(3).map(Path::to_path_buf))
        .unwrap_or_else(|| {
            let target_dir = env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| manifest_dir.join("target"));

            let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_owned());

            target_dir.join(profile)
        });

    fs::create_dir_all(&out_dir).expect("create output dir");

    fs::copy(&source, out_dir.join("csigner.bin")).expect("copy csigner library");

    println!("cargo:rerun-if-changed={}", source.display());

    // sign.bin
    let sign_bin = resources_dir.join(bin_filename);

    if !sign_bin.is_file() {
        println!(
            "cargo:warning=csigner: 未找到附加bundle库 {}",
            sign_bin.display()
        );
    }

    if sign_bin.is_file() {
        fs::copy(&sign_bin, out_dir.join("sign.bin")).expect("copy sign.bin");

        println!("cargo:rerun-if-changed={}", sign_bin.display());
    }

    println!(
        "cargo:warning=csigner: 已部署 {} -> {}",
        source.display(),
        out_dir.display()
    );
}

fn emit_librespot_env() {
    println!("cargo:rustc-env=VERGEN_GIT_SHA=vendored");
    println!("cargo:rustc-env=VERGEN_GIT_COMMIT_DATE=vendored");
    println!("cargo:rustc-env=VERGEN_BUILD_DATE=vendored");
    println!("cargo:rustc-env=LIBRESPOT_BUILD_ID=mineradio");
}

fn compile_librespot_protocol() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let proto_dir = manifest_dir
        .join("src")
        .join("vendor")
        .join("librespot_protocol")
        .join("proto");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let _ = fs::remove_dir_all(&out_dir);
    fs::create_dir_all(&out_dir).expect("create OUT_DIR");

    let files = [
        "connect.proto",
        "media.proto",
        "connectivity.proto",
        "devices.proto",
        "entity_extension_data.proto",
        "extended_metadata.proto",
        "extension_kind.proto",
        "metadata.proto",
        "player.proto",
        "playlist_annotate3.proto",
        "playlist_permission.proto",
        "playlist4_external.proto",
        "lens-model.proto",
        "signal-model.proto",
        "spotify/clienttoken/v0/clienttoken_http.proto",
        "spotify/login5/v3/challenges/code.proto",
        "spotify/login5/v3/challenges/hashcash.proto",
        "spotify/login5/v3/client_info.proto",
        "spotify/login5/v3/credentials/credentials.proto",
        "spotify/login5/v3/identifiers/identifiers.proto",
        "spotify/login5/v3/login5.proto",
        "spotify/login5/v3/user_info.proto",
        "storage-resolve.proto",
        "user_attributes.proto",
        "autoplay_context_request.proto",
        "social_connect_v2.proto",
        "transfer_state.proto",
        "context_player_options.proto",
        "playback.proto",
        "play_history.proto",
        "session.proto",
        "queue.proto",
        "context_track.proto",
        "context.proto",
        "restrictions.proto",
        "context_page.proto",
        "play_origin.proto",
        "suppressions.proto",
        "instrumentation_params.proto",
        "authentication.proto",
        "canvaz.proto",
        "canvaz-meta.proto",
        "explicit_content_pubsub.proto",
        "keyexchange.proto",
        "mercury.proto",
        "pubsub.proto",
    ]
    .map(|file| proto_dir.join(file));

    let inputs = files.iter().map(PathBuf::as_path).collect::<Vec<&Path>>();

    protobuf_codegen::Codegen::new()
        .pure()
        .out_dir(&out_dir)
        .inputs(&inputs)
        .include(&proto_dir)
        .run()
        .expect("compile librespot protobuf");
}
