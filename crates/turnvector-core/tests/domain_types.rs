use turnvector_core::{
    ByteCount, CommandId, ConnectionId, DaemonInstanceId, DomainValueError, Duration,
    EventSequence, ModelId, MonotonicTime, OperationId, OutputSequence, RequestId, RequestSequence,
    RequestStatusVersion, TokenCount,
};

#[test]
fn identities_reject_zero_and_request_identity_preserves_ownership() {
    assert_eq!(DaemonInstanceId::new(0), Err(DomainValueError::Zero));
    assert_eq!(ConnectionId::new(0), Err(DomainValueError::Zero));
    assert_eq!(ModelId::new(0), Err(DomainValueError::Zero));
    assert_eq!(OperationId::new(0), Err(DomainValueError::Zero));

    let daemon = DaemonInstanceId::new(1).unwrap();
    let connection = ConnectionId::new(2).unwrap();
    let sequence = RequestSequence::new(3).unwrap();
    let request = RequestId::new(daemon, connection, sequence);

    assert_eq!(request.daemon_instance(), daemon);
    assert_eq!(request.connection(), connection);
    assert_eq!(request.sequence(), sequence);
}

#[test]
fn sequences_are_nonzero_and_never_wrap() {
    assert_eq!(CommandId::new(0), Err(DomainValueError::Zero));
    assert_eq!(EventSequence::new(0), Err(DomainValueError::Zero));
    assert_eq!(OutputSequence::new(0), Err(DomainValueError::Zero));
    assert_eq!(RequestStatusVersion::new(0), Err(DomainValueError::Zero));

    assert_eq!(
        EventSequence::new(u64::MAX).unwrap().next(),
        Err(DomainValueError::Overflow)
    );
    assert_eq!(
        RequestStatusVersion::new(1).unwrap().next().unwrap().get(),
        2
    );
}

#[test]
fn units_use_checked_arithmetic() {
    assert_eq!(ByteCount::new(0).get(), 0);
    assert_eq!(TokenCount::new(0).get(), 0);
    assert_eq!(
        ByteCount::new(u64::MAX).checked_add(ByteCount::new(1)),
        Err(DomainValueError::Overflow)
    );
    assert_eq!(
        TokenCount::new(1).checked_sub(TokenCount::new(2)),
        Err(DomainValueError::Underflow)
    );
}

#[test]
fn duration_uses_checked_subtraction_with_typed_underflow() {
    let max = Duration::from_micros(u64::MAX);
    assert_eq!(
        Duration::from_micros(5).checked_sub(Duration::from_micros(2)),
        Ok(Duration::from_micros(3))
    );
    assert_eq!(
        Duration::from_micros(0).checked_sub(Duration::from_micros(0)),
        Ok(Duration::from_micros(0))
    );
    assert_eq!(max.checked_sub(Duration::from_micros(0)), Ok(max));
    assert_eq!(max.checked_sub(max), Ok(Duration::from_micros(0)));
    assert_eq!(
        Duration::from_micros(0).checked_sub(Duration::from_micros(1)),
        Err(DomainValueError::Underflow)
    );
    assert_eq!(
        Duration::from_micros(1).checked_sub(Duration::from_micros(2)),
        Err(DomainValueError::Underflow)
    );
    assert_eq!(
        max.checked_add(Duration::from_micros(1)),
        Err(DomainValueError::Overflow)
    );
}

#[test]
fn monotonic_time_uses_typed_checked_durations() {
    let start = MonotonicTime::from_micros(10);
    let elapsed = Duration::from_micros(5);
    let end = start.checked_add(elapsed).unwrap();

    assert_eq!(end.as_micros(), 15);
    assert_eq!(end.checked_duration_since(start).unwrap(), elapsed);
    assert_eq!(
        start.checked_duration_since(end),
        Err(DomainValueError::Underflow)
    );
    assert_eq!(
        MonotonicTime::from_micros(u64::MAX).checked_add(Duration::from_micros(1)),
        Err(DomainValueError::Overflow)
    );
}
