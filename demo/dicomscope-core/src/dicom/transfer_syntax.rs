//! What this build can and cannot decode, and why. The README table is
//! generated from the same list so the two cannot drift apart.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// Decoded successfully against a real file.
    Verified,
    /// Same pure-Rust code path as a verified syntax; expected to work.
    Expected,
    /// Cannot be decoded in the browser build, with the reason.
    Unsupported(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub struct TransferSyntaxInfo {
    pub uid: &'static str,
    pub name: &'static str,
    pub support: Support,
}

pub const JPEG2000_REASON: &str = "the only pure-Rust JPEG 2000 decoder (openjp2) declares itself \
    as a cdylib and fails to link against wasm32-unknown-unknown because it references libc's free";

pub const TABLE: &[TransferSyntaxInfo] = &[
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2",
        name: "Implicit VR Little Endian",
        support: Support::Expected,
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.1",
        name: "Explicit VR Little Endian",
        support: Support::Verified,
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.1.99",
        name: "Deflated Explicit VR Little Endian",
        support: Support::Expected,
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.2",
        name: "Explicit VR Big Endian (retired)",
        support: Support::Expected,
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.5",
        name: "RLE Lossless",
        support: Support::Verified,
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.4.50",
        name: "JPEG Baseline (Process 1)",
        support: Support::Expected,
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.4.51",
        name: "JPEG Extended (Process 2 & 4)",
        support: Support::Expected,
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.4.57",
        name: "JPEG Lossless (Process 14)",
        support: Support::Expected,
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.4.70",
        name: "JPEG Lossless (Process 14, SV1)",
        support: Support::Verified,
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.4.80",
        name: "JPEG-LS Lossless",
        support: Support::Unsupported("no pure-Rust JPEG-LS decoder is wired into dicom-pixeldata"),
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.4.81",
        name: "JPEG-LS Near-Lossless",
        support: Support::Unsupported("no pure-Rust JPEG-LS decoder is wired into dicom-pixeldata"),
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.4.90",
        name: "JPEG 2000 Lossless",
        support: Support::Unsupported(JPEG2000_REASON),
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.4.91",
        name: "JPEG 2000",
        support: Support::Unsupported(JPEG2000_REASON),
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.4.201",
        name: "HTJ2K Lossless",
        support: Support::Unsupported(JPEG2000_REASON),
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.4.202",
        name: "HTJ2K Lossless RPCL",
        support: Support::Unsupported(JPEG2000_REASON),
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.4.203",
        name: "HTJ2K",
        support: Support::Unsupported(JPEG2000_REASON),
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.4.100",
        name: "MPEG2 Main Profile",
        support: Support::Unsupported("video transfer syntaxes are out of scope"),
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.4.102",
        name: "MPEG-4 AVC/H.264",
        support: Support::Unsupported("video transfer syntaxes are out of scope"),
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.4.110",
        name: "JPEG XL Lossless",
        support: Support::Unsupported("the JPEG XL decoder is not enabled in this build"),
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.4.111",
        name: "JPEG XL Recompression",
        support: Support::Unsupported("the JPEG XL decoder is not enabled in this build"),
    },
    TransferSyntaxInfo {
        uid: "1.2.840.10008.1.2.4.112",
        name: "JPEG XL",
        support: Support::Unsupported("the JPEG XL decoder is not enabled in this build"),
    },
];

/// Look up a transfer syntax UID. Trailing NULs and whitespace are ignored
/// because UI strings are padded to even length in files.
pub fn lookup(uid: &str) -> Option<&'static TransferSyntaxInfo> {
    let uid = uid.trim_end_matches('\0').trim();
    TABLE.iter().find(|t| t.uid == uid)
}

/// Name and reason when the syntax is known to be unsupported.
pub fn unsupported(uid: &str) -> Option<(&'static str, &'static str)> {
    match lookup(uid)?.support {
        Support::Unsupported(reason) => Some((lookup(uid)?.name, reason)),
        _ => None,
    }
}

/// Human readable name, or the UID itself when unknown.
pub fn name(uid: &str) -> &str {
    lookup(uid).map(|t| t.name).unwrap_or(uid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_uid_is_found() {
        assert_eq!(
            lookup("1.2.840.10008.1.2.5\0").map(|t| t.name),
            Some("RLE Lossless")
        );
        assert_eq!(
            lookup("1.2.840.10008.1.2.4.90 ").map(|t| t.support),
            Some(Support::Unsupported(JPEG2000_REASON))
        );
    }

    #[test]
    fn unsupported_gives_reason() {
        let (name, reason) = unsupported("1.2.840.10008.1.2.4.91").unwrap();
        assert_eq!(name, "JPEG 2000");
        assert!(reason.contains("openjp2"));
        assert!(unsupported("1.2.840.10008.1.2.1").is_none());
        assert!(unsupported("9.9.9").is_none());
    }

    #[test]
    fn uids_are_unique() {
        for (i, a) in TABLE.iter().enumerate() {
            assert!(!TABLE[i + 1..].iter().any(|b| b.uid == a.uid), "{}", a.uid);
        }
    }
}
