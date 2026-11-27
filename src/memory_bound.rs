//! Declared resident-memory bound and its derived payload budget.
//!
//! Operators declare a resident-set bound (`--memory-bound-bytes`) and an
//! explicit headroom (`--memory-bound-headroom-bytes`). The bound must fit
//! inside the readable cgroup limit when one exists, the headroom must be
//! positive and smaller than the bound, and the process baseline resident set
//! measured at startup must leave room for the payload budget. The derived
//! payload limit is the declared bound minus headroom minus baseline, capped
//! by the configured `--max-data-bytes`.
//!
//! Enforcement stays on the existing payload-admission path: every mutation is
//! refused before it can grow the payload past the derived limit. Resident set
//! is *bounded*, not hard-capped: stacks, allocator metadata, socket buffers
//! and fragmentation are covered by the declared headroom, and the operator's
//! cgroup limit remains the kernel-enforced backstop. See
//! `docs/production-profile.md` for the headroom guidance.

/// Declared bound inputs. `bound_bytes` and `headroom_bytes` are optional only
/// so the unconfigured default keeps the historical payload-only budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryBoundRequest {
    pub bound_bytes: Option<u64>,
    pub headroom_bytes: Option<u64>,
    /// The payload limit that would otherwise apply (`--max-data-bytes`).
    pub payload_limit_bytes: u64,
}

/// Startup observations used to validate and derive the payload budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryObservations {
    /// Resident set of this process at startup; `None` when unreadable.
    pub resident_bytes: Option<u64>,
    /// Readable cgroup limit for this process; `None` when absent or `max`.
    pub cgroup_limit_bytes: Option<u64>,
}

/// Resolved bound reported by `INFO memory`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryBound {
    pub bound_bytes: u64,
    pub headroom_bytes: u64,
    pub baseline_bytes: u64,
    pub cgroup_limit_bytes: Option<u64>,
    pub payload_limit_bytes: u64,
}

/// Resolve the declared bound, or `Ok(None)` when no bound was requested.
///
/// Every rejection is fail-closed: the process must never start with a bound it
/// cannot honour.
pub fn resolve(
    request: MemoryBoundRequest,
    observations: MemoryObservations,
) -> Result<Option<MemoryBound>, String> {
    let bound = match (request.bound_bytes, request.headroom_bytes) {
        (None, None) => return Ok(None),
        (Some(_), None) | (None, Some(_)) => {
            return Err(
                "--memory-bound-bytes and --memory-bound-headroom-bytes must be given together"
                    .into(),
            );
        }
        (Some(bound), Some(headroom)) => (bound, headroom),
    };
    let (bound_bytes, headroom_bytes) = bound;
    if bound_bytes == 0 {
        return Err("--memory-bound-bytes must be positive".into());
    }
    if headroom_bytes == 0 {
        return Err("--memory-bound-headroom-bytes must be positive".into());
    }
    if headroom_bytes >= bound_bytes {
        return Err(
            "--memory-bound-headroom-bytes must be smaller than --memory-bound-bytes".into(),
        );
    }
    if let Some(limit) = observations.cgroup_limit_bytes {
        if bound_bytes > limit {
            return Err(format!(
                "--memory-bound-bytes {bound_bytes} exceeds the cgroup limit {limit}"
            ));
        }
    }
    let baseline_bytes = observations.resident_bytes.ok_or(
        "--memory-bound-bytes requires a readable process resident set; not supported here",
    )?;
    let available = bound_bytes - headroom_bytes;
    if baseline_bytes >= available {
        return Err(format!(
            "--memory-bound-headroom-bytes {headroom_bytes} leaves no payload budget: \
             baseline resident set {baseline_bytes} already reaches {available}"
        ));
    }
    let derived = available - baseline_bytes;
    Ok(Some(MemoryBound {
        bound_bytes,
        headroom_bytes,
        baseline_bytes,
        cgroup_limit_bytes: observations.cgroup_limit_bytes,
        payload_limit_bytes: request.payload_limit_bytes.min(derived),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    fn request(bound: Option<u64>, headroom: Option<u64>) -> MemoryBoundRequest {
        MemoryBoundRequest {
            bound_bytes: bound,
            headroom_bytes: headroom,
            payload_limit_bytes: 64 * MIB,
        }
    }

    fn observations(resident: Option<u64>, cgroup: Option<u64>) -> MemoryObservations {
        MemoryObservations {
            resident_bytes: resident,
            cgroup_limit_bytes: cgroup,
        }
    }

    #[test]
    fn unconfigured_keeps_the_payload_only_budget() {
        assert_eq!(
            resolve(request(None, None), observations(Some(10 * MIB), None)),
            Ok(None)
        );
    }

    #[test]
    fn rejects_a_bound_without_headroom() {
        assert!(
            resolve(
                request(Some(100 * MIB), None),
                observations(Some(10 * MIB), None)
            )
            .is_err()
        );
        assert!(
            resolve(
                request(None, Some(10 * MIB)),
                observations(Some(10 * MIB), None)
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_zero_and_inverted_headroom() {
        assert!(resolve(request(Some(0), Some(MIB)), observations(Some(MIB), None)).is_err());
        assert!(
            resolve(
                request(Some(10 * MIB), Some(0)),
                observations(Some(MIB), None)
            )
            .is_err()
        );
        assert!(
            resolve(
                request(Some(10 * MIB), Some(10 * MIB)),
                observations(Some(MIB), None)
            )
            .is_err()
        );
        assert!(
            resolve(
                request(Some(10 * MIB), Some(11 * MIB)),
                observations(Some(MIB), None)
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_a_bound_above_the_cgroup_limit() {
        assert!(
            resolve(
                request(Some(100 * MIB), Some(10 * MIB)),
                observations(Some(10 * MIB), Some(64 * MIB))
            )
            .is_err()
        );
        let bound = resolve(
            request(Some(64 * MIB), Some(10 * MIB)),
            observations(Some(10 * MIB), Some(64 * MIB)),
        )
        .expect("bound inside the cgroup limit");
        assert_eq!(bound.expect("configured").payload_limit_bytes, 44 * MIB);
    }

    #[test]
    fn rejects_an_impossible_headroom_against_the_baseline() {
        assert!(
            resolve(
                request(Some(64 * MIB), Some(8 * MIB)),
                observations(Some(60 * MIB), None)
            )
            .is_err()
        );
    }

    #[test]
    fn fails_closed_when_resident_set_is_unreadable() {
        assert!(
            resolve(
                request(Some(64 * MIB), Some(8 * MIB)),
                observations(None, None)
            )
            .is_err()
        );
    }

    #[test]
    fn caps_the_payload_limit_at_the_derived_budget() {
        let bound = resolve(
            request(Some(64 * MIB), Some(8 * MIB)),
            observations(Some(20 * MIB), None),
        )
        .expect("valid bound")
        .expect("configured");
        assert_eq!(bound.payload_limit_bytes, 36 * MIB);
        assert_eq!(bound.baseline_bytes, 20 * MIB);

        let small = resolve(
            MemoryBoundRequest {
                bound_bytes: Some(64 * MIB),
                headroom_bytes: Some(8 * MIB),
                payload_limit_bytes: 4 * MIB,
            },
            observations(Some(20 * MIB), None),
        )
        .expect("valid bound")
        .expect("configured");
        assert_eq!(small.payload_limit_bytes, 4 * MIB);
    }
}

/// Read the startup observations used by [`resolve`] from this process.
///
/// Resident set and cgroup limits are Linux-only observations; both map to
/// `None` elsewhere so a declared bound fails closed instead of pretending to
/// be enforceable.
pub fn observe() -> MemoryObservations {
    let resident_bytes =
        crate::engine::process_memory::snapshot().map(|memory| memory.resident_bytes);
    let cgroup_limit_bytes =
        crate::engine::process_memory::cgroup_snapshot().and_then(|memory| memory.limit_bytes);
    MemoryObservations {
        resident_bytes,
        cgroup_limit_bytes,
    }
}
