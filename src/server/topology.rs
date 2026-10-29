//! Conservative bind-time SMT hint, not affinity or physical-core accounting.
//! Missing/unsupported status keeps one coordinator; no host settings are written.

pub(super) fn smt_disabled() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::fs::File::open("/sys/devices/system/cpu/smt/active").is_ok_and(read_smt_disabled)
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(any(target_os = "linux", test))]
fn read_smt_disabled(reader: impl std::io::Read) -> bool {
    // The kernel emits exactly "0\n" or "1\n". Read one extra byte to reject
    // malformed/oversized input without an unbounded sysfs read.
    use std::io::Read;
    let mut value = Vec::with_capacity(3);
    reader.take(3).read_to_end(&mut value).is_ok() && value == b"0\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Read};

    #[test]
    fn only_an_explicit_complete_disabled_status_allows_two_coordinators() {
        assert!(read_smt_disabled(b"0\n".as_slice()));
        for value in [
            b"".as_slice(),
            b"0",
            b"1\n",
            b"2\n",
            b"0\nextra",
            b"00\n",
            b" 0\n",
        ] {
            assert!(!read_smt_disabled(value), "{value:?}");
        }
        let mut oversized = b"0\nextra".as_slice();
        assert!(!read_smt_disabled(&mut oversized));
        assert_eq!(oversized, b"xtra", "read at most three bytes");
    }

    #[test]
    fn short_reads_work_and_read_failures_fall_back_conservatively() {
        struct Bytewise<'a>(&'a [u8]);
        impl Read for Bytewise<'_> {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                let count = output.len().min(1);
                self.0.read(&mut output[..count])
            }
        }
        struct Failed;
        impl Read for Failed {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
            }
        }
        assert!(read_smt_disabled(Bytewise(b"0\n")));
        assert!(!read_smt_disabled(Bytewise(b"1\n")));
        assert!(!read_smt_disabled(Failed));
    }
}
