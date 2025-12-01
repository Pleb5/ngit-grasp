//! Git Smart HTTP Protocol Implementation
//!
//! This module implements the Git pkt-line format and protocol utilities.
//!
//! # Pkt-line Format
//!
//! A pkt-line is a variable length binary string with a 4-byte length prefix:
//! - First 4 bytes: hex digits representing total length (including these 4 bytes)
//! - Remaining bytes: payload data
//! - Special case "0000": flush packet (end of section)
//!
//! # References
//! - https://git-scm.com/docs/protocol-common#_pkt_line_format

use std::fmt;

/// Represents a Git pkt-line packet
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PktLine {
    /// Data packet with payload
    Data(Vec<u8>),
    /// Flush packet (0000)
    Flush,
}

impl PktLine {
    /// Create a data packet from bytes
    pub fn data(data: impl Into<Vec<u8>>) -> Self {
        Self::Data(data.into())
    }

    /// Create a flush packet
    pub fn flush() -> Self {
        Self::Flush
    }

    /// Encode this packet to wire format
    pub fn encode(&self) -> Vec<u8> {
        match self {
            PktLine::Flush => b"0000".to_vec(),
            PktLine::Data(data) => {
                let len = data.len() + 4;
                let mut result = Vec::with_capacity(len);
                result.extend_from_slice(format!("{:04x}", len).as_bytes());
                result.extend_from_slice(data);
                result
            }
        }
    }

    /// Parse a single pkt-line from bytes
    /// Returns (packet, remaining_bytes)
    pub fn parse(input: &[u8]) -> Result<(Self, &[u8]), ProtocolError> {
        if input.len() < 4 {
            return Err(ProtocolError::InsufficientData);
        }

        let len_str =
            std::str::from_utf8(&input[0..4]).map_err(|_| ProtocolError::InvalidLength)?;

        let len =
            u16::from_str_radix(len_str, 16).map_err(|_| ProtocolError::InvalidLength)? as usize;

        if len == 0 {
            // Flush packet
            return Ok((PktLine::Flush, &input[4..]));
        }

        if len < 4 {
            return Err(ProtocolError::InvalidLength);
        }

        if input.len() < len {
            return Err(ProtocolError::InsufficientData);
        }

        let data = input[4..len].to_vec();
        Ok((PktLine::Data(data), &input[len..]))
    }

    /// Parse all pkt-lines from bytes
    pub fn parse_all(mut input: &[u8]) -> Result<Vec<Self>, ProtocolError> {
        let mut packets = Vec::new();

        while !input.is_empty() {
            let (packet, remaining) = Self::parse(input)?;
            let is_flush = matches!(packet, PktLine::Flush);
            packets.push(packet);
            input = remaining;

            // Stop at flush packet
            if is_flush {
                break;
            }
        }

        Ok(packets)
    }
}

/// Errors that can occur during protocol parsing
#[derive(Debug)]
pub enum ProtocolError {
    /// Not enough data to parse a complete packet
    InsufficientData,
    /// Invalid length prefix
    InvalidLength,
    /// Invalid UTF-8 in packet data
    InvalidUtf8,
    /// IO error
    Io(std::io::Error),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientData => write!(f, "insufficient data for pkt-line"),
            Self::InvalidLength => write!(f, "invalid pkt-line length"),
            Self::InvalidUtf8 => write!(f, "invalid UTF-8 in pkt-line"),
            Self::Io(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<std::io::Error> for ProtocolError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Git service type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitService {
    /// Upload pack (clone/fetch)
    UploadPack,
    /// Receive pack (push)
    ReceivePack,
}

impl GitService {
    /// Parse service from query parameter
    pub fn from_query_param(service: &str) -> Option<Self> {
        match service {
            "git-upload-pack" => Some(Self::UploadPack),
            "git-receive-pack" => Some(Self::ReceivePack),
            _ => None,
        }
    }

    /// Get the service name as used in Git protocol
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UploadPack => "git-upload-pack",
            Self::ReceivePack => "git-receive-pack",
        }
    }

    /// Get the git command name (without "git-" prefix) for subprocess invocation
    pub fn command_name(&self) -> &'static str {
        match self {
            Self::UploadPack => "upload-pack",
            Self::ReceivePack => "receive-pack",
        }
    }

    /// Get the content type for the service advertisement
    pub fn advertisement_content_type(&self) -> &'static str {
        match self {
            Self::UploadPack => "application/x-git-upload-pack-advertisement",
            Self::ReceivePack => "application/x-git-receive-pack-advertisement",
        }
    }

    /// Get the content type for the service result
    pub fn result_content_type(&self) -> &'static str {
        match self {
            Self::UploadPack => "application/x-git-upload-pack-result",
            Self::ReceivePack => "application/x-git-receive-pack-result",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pktline_encode_flush() {
        let pkt = PktLine::flush();
        assert_eq!(pkt.encode(), b"0000");
    }

    #[test]
    fn test_pktline_encode_data() {
        let pkt = PktLine::data(b"hello");
        assert_eq!(pkt.encode(), b"0009hello");
    }

    #[test]
    fn test_pktline_parse_flush() {
        let (pkt, remaining) = PktLine::parse(b"0000extra").unwrap();
        assert_eq!(pkt, PktLine::Flush);
        assert_eq!(remaining, b"extra");
    }

    #[test]
    fn test_pktline_parse_data() {
        let (pkt, remaining) = PktLine::parse(b"0009helloworld").unwrap();
        assert_eq!(pkt, PktLine::data(b"hello"));
        assert_eq!(remaining, b"world");
    }

    #[test]
    fn test_pktline_parse_insufficient_data() {
        let result = PktLine::parse(b"000");
        assert!(matches!(result, Err(ProtocolError::InsufficientData)));
    }

    #[test]
    fn test_pktline_parse_invalid_length() {
        let result = PktLine::parse(b"xxxx");
        assert!(matches!(result, Err(ProtocolError::InvalidLength)));
    }

    #[test]
    fn test_pktline_parse_all() {
        let input = b"0009hello000aworld\n0000";
        let packets = PktLine::parse_all(input).unwrap();
        assert_eq!(packets.len(), 3);
        assert_eq!(packets[0], PktLine::data(b"hello"));
        assert_eq!(packets[1], PktLine::data(b"world\n"));
        assert_eq!(packets[2], PktLine::Flush);
    }

    #[test]
    fn test_git_service_from_query() {
        assert_eq!(
            GitService::from_query_param("git-upload-pack"),
            Some(GitService::UploadPack)
        );
        assert_eq!(
            GitService::from_query_param("git-receive-pack"),
            Some(GitService::ReceivePack)
        );
        assert_eq!(GitService::from_query_param("invalid"), None);
    }

    #[test]
    fn test_git_service_content_types() {
        let upload = GitService::UploadPack;
        assert_eq!(
            upload.advertisement_content_type(),
            "application/x-git-upload-pack-advertisement"
        );
        assert_eq!(
            upload.result_content_type(),
            "application/x-git-upload-pack-result"
        );
    }
}
