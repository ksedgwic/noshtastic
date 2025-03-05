/// Link frame definition
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct LinkFrame {
    /// 'NOSH'
    #[prost(fixed32, tag = "1")]
    pub magic: u32,
    #[prost(uint32, tag = "2")]
    pub version: u32,
    #[prost(oneof = "link_frame::Payload", tags = "3, 4, 5")]
    pub payload: ::core::option::Option<link_frame::Payload>,
}
/// Nested message and enum types in `LinkFrame`.
pub mod link_frame {
    #[allow(clippy::derive_partial_eq_without_eq)]
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Payload {
        /// A complete, unfragmented message
        #[prost(message, tag = "3")]
        Complete(super::LinkMsg),
        /// A fragment of a message
        #[prost(message, tag = "4")]
        Fragment(super::LinkFrag),
        /// Requests missing fragment resend
        #[prost(message, tag = "5")]
        Missing(super::LinkMissing),
    }
}
/// A complete link message (not fragmented)
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct LinkMsg {
    /// Truncated nostr msgid (or hash for others)
    #[prost(fixed64, tag = "1")]
    pub msgid: u64,
    #[prost(bytes = "vec", tag = "2")]
    pub data: ::prost::alloc::vec::Vec<u8>,
}
/// A fragment of a link message
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct LinkFrag {
    /// Truncated nostr msgid (or hash for others)
    #[prost(fixed64, tag = "1")]
    pub msgid: u64,
    /// Total number of fragments in the packet
    #[prost(uint32, tag = "2")]
    pub numfrag: u32,
    /// Index of this fragment (starting at 0)
    #[prost(uint32, tag = "3")]
    pub fragndx: u32,
    /// Octet rotation offset (see noshtastic:#15)
    #[prost(uint32, tag = "4")]
    pub rotoff: u32,
    /// Raw data buffer for this fragment
    #[prost(bytes = "vec", tag = "5")]
    pub data: ::prost::alloc::vec::Vec<u8>,
}
/// A missing fragment resend request
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct LinkMissing {
    /// Truncated nostr msgid (or hash for others)
    #[prost(fixed64, tag = "1")]
    pub msgid: u64,
    /// List of all missing fragments
    #[prost(uint32, repeated, tag = "2")]
    pub fragndx: ::prost::alloc::vec::Vec<u32>,
}
