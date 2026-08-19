use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    emit_librespot_env();
    compile_librespot_protocol();
    deploy_csigner_resources();
}

/// 按当前打包目标平台从 `resources/<target>/` 筛选 csigner 动态库，复制到**业务方输出目录**
/// （业务方最终产物所在，即 `target/<profile>/`），统一命名；只要找到动态库就无条件附带
/// 库文件统一为 `csigner.dll` / `libcsigner.so` / `libcsigner.dylib`。
fn deploy_csigner_resources() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let target = env::var("TARGET").expect("TARGET");
    println!("cargo:target={}", target);
    // 目标平台 -> 源动态库扩展名 / 统一目标名
    let (ext, dest_name) = if target.contains("windows") {
        ("dll", "csigner.dll")
    } else if target.contains("linux") {
        ("so", "libcsigner.so")
    } else if target.contains("apple") {
        ("dylib", "libcsigner.dylib")
    } else {
        return;
    };

    let resources_dir = manifest_dir.join("resources");
    let platform_dir = resources_dir.join(&target);
    let Some(source) = find_platform_dll(&platform_dir, ext) else {
        println!(
            "cargo:warning=csigner: 未找到目标平台 {target} 的动态库（{}），跳过复制",
            platform_dir.display()
        );
        return;
    };

    // 业务方输出目录：`CARGO_TARGET_DIR` 不会传给依赖的 build.rs（实测为未设置），
    // 因此用 `OUT_DIR` 推导。`OUT_DIR = <target>/<profile>/build/<pkg>-<hash>/out`，
    // 向上 3 级即 `<target>/<profile>`，正是业务方最终产物所在目录。
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

    fs::copy(&source, out_dir.join(&dest_name)).expect("copy csigner library");
    println!("cargo:rerun-if-changed={}", source.display());

    // sign.bin：只要找到对应目标平台的动态库就无条件复制
    let sign_bin = resources_dir.join("sign.bin");
    if sign_bin.is_file() {
        fs::copy(&sign_bin, out_dir.join("sign.bin")).expect("copy sign.bin");
        println!("cargo:rerun-if-changed={}", sign_bin.display());
    }

    // 告知运行时库文件名（编译期常量，Rust 侧用 option_env! 读取）
    println!("cargo:rustc-env=CSIGNER_LIB_FILENAME={dest_name}");
    println!(
        "cargo:warning=csigner: 已部署 {dest_name} -> {}",
        out_dir.display()
    );
}

/// 在平台目录里找第一个扩展名匹配的动态库
/// （mingw 产物可能带 `lib` 前缀如 `libcsigner.dll`，MSVC 直接叫 `csigner.dll`）。
fn find_platform_dll(dir: &Path, ext: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case(ext))
        })
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
