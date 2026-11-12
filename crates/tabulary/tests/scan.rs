use tabulary::{BuildError, ScanSegment};

#[test]
fn invalid_segments_are_structured_build_errors() {
    for (segment, total) in [(0, 0), (1, 1), (3, 2), (0, 1_000_001), (u32::MAX, u32::MAX)] {
        assert!(matches!(
            ScanSegment::new(segment, total),
            Err(BuildError::OperationConstraint {
                code: "invalid_scan_segment"
            })
        ));
    }
}

#[test]
fn valid_boundary_segments_are_accepted() {
    for (segment, total) in [(0, 1), (0, 1_000_000), (999_999, 1_000_000)] {
        let value = ScanSegment::new(segment, total).unwrap();
        assert_eq!(value, ScanSegment::new(segment, total).unwrap());
    }
}
