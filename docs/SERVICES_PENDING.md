# Pending Service Verification

## Podcast

- [ ] Run end-to-end checks with real podcast audio URLs.
- [ ] Compare Rust beatmap output with the historical JS analyzer on representative
  samples.
- [ ] Confirm long-audio fallback behavior under real network conditions.

## Discover Home

- [ ] Verify API behavior across real provider and login-session states.

## Shared Playlist Import

- [ ] Verify Apple Music and Kugou imports with real public share links.
- [ ] Confirm front-end compatibility of import-only track payloads.

## Other services

- [ ] Run broader real-world verification for audio proxy, image proxy, Soda audio
  proxy, cross-source resolver, and weather radio.

## Known technical TODO

- [ ] Add an EAPI decompressed-size limit in
  `src/utils/cryptors/netease.rs` after confirming the real payload-size range.
