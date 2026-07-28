use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    emit_librespot_env();
    compile_librespot_protocol();
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
