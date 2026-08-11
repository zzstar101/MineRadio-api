//! # 注意
//!
//! 下称Netease为N
//!
//! 由于N的推荐页面返回的卡片需要被硬划分回playlist, stream, track
//! 而获取详情的接口没有预留额外参数位置
//! 所以选择将推荐页的卡片id设置为头标识符并拼接所有参数, 再由接口动态解析
pub mod adapter;
pub mod client;
pub mod crypto;
mod lyric;
pub mod map;
mod model;
