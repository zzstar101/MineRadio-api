use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use crate::librespot_metadata::util::{impl_deref_wrapped, impl_from_repeated};

use crate::librespot_core::FileId;

use crate::librespot_protocol as protocol;
use protocol::metadata::VideoFile as VideoFileMessage;

#[derive(Debug, Clone, Default)]
pub struct VideoFiles(pub Vec<FileId>);

impl_deref_wrapped!(VideoFiles, Vec<FileId>);

impl_from_repeated!(VideoFileMessage, VideoFiles);
