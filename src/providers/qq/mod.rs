/*
注意: 
1.qq的在线接口不提供是否收藏该项目
2.qq每一个项目都有两个标识符(id: u32, mid: String), 此api项目只使用mid
*/
pub mod adapter;
pub mod client;
pub mod map;
mod model;
