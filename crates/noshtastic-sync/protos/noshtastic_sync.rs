#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SyncMessage {
    /// Protocol version
    #[prost(uint32, tag = "1")]
    pub version: u32,
    #[prost(oneof = "sync_message::Payload", tags = "2, 3, 4, 5, 6")]
    pub payload: ::core::option::Option<sync_message::Payload>,
}
/// Nested message and enum types in `SyncMessage`.
pub mod sync_message {
    #[allow(clippy::derive_partial_eq_without_eq)]
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Payload {
        #[prost(message, tag = "2")]
        Ping(super::Ping),
        #[prost(message, tag = "3")]
        Pong(super::Pong),
        #[prost(message, tag = "4")]
        Negentropy(super::NegentropyMessage),
        #[prost(message, tag = "5")]
        RawNote(super::RawNote),
        #[prost(message, tag = "6")]
        EncNote(super::EncNote),
    }
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Ping {
    #[prost(uint32, tag = "1")]
    pub id: u32,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Pong {
    #[prost(uint32, tag = "1")]
    pub id: u32,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct NegentropyMessage {
    #[prost(bytes = "vec", tag = "1")]
    pub data: ::prost::alloc::vec::Vec<u8>,
}
/// utf8 encoded json
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RawNote {
    #[prost(bytes = "vec", tag = "1")]
    pub data: ::prost::alloc::vec::Vec<u8>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EncNote {
    #[prost(bytes = "vec", tag = "1")]
    pub id: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub pubkey: ::prost::alloc::vec::Vec<u8>,
    #[prost(uint64, tag = "3")]
    pub created_at: u64,
    #[prost(uint32, tag = "4")]
    pub kind: u32,
    #[prost(message, repeated, tag = "5")]
    pub tags: ::prost::alloc::vec::Vec<EncTag>,
    #[prost(message, optional, tag = "6")]
    pub content: ::core::option::Option<EncString>,
    #[prost(bytes = "vec", tag = "7")]
    pub sig: ::prost::alloc::vec::Vec<u8>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EncTag {
    #[prost(message, repeated, tag = "1")]
    pub elements: ::prost::alloc::vec::Vec<EncString>,
}
/// used when the string might be a lower-case hex string (or not)
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EncString {
    #[prost(oneof = "enc_string::StringType", tags = "1, 2, 3")]
    pub string_type: ::core::option::Option<enc_string::StringType>,
}
/// Nested message and enum types in `EncString`.
pub mod enc_string {
    #[allow(clippy::derive_partial_eq_without_eq)]
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum StringType {
        /// utf8 string
        #[prost(string, tag = "1")]
        Utf8String(::prost::alloc::string::String),
        /// Hex-encoded data stored as bytes
        #[prost(bytes, tag = "2")]
        HexEncoded(::prost::alloc::vec::Vec<u8>),
        #[prost(bytes, tag = "3")]
        Compressed(::prost::alloc::vec::Vec<u8>),
    }
}
