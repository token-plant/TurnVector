use turnvector_core::{
    BackendGeneration, BoundedCollectionError, BoundedMap, BoundedSet, BoundedVec,
    DomainValueError, GenerationComponent, GenerationMismatch, GenerationVector,
    RuntimeOverheadGeneration, SafetyGeneration, SchedulerGeneration,
};

fn generations(values: [u64; 4]) -> GenerationVector {
    GenerationVector::new(
        SchedulerGeneration::new(values[0]).unwrap(),
        BackendGeneration::new(values[1]).unwrap(),
        SafetyGeneration::new(values[2]).unwrap(),
        RuntimeOverheadGeneration::new(values[3]).unwrap(),
    )
}

#[test]
fn every_generation_mismatch_makes_the_vector_stale() {
    let planned = generations([1, 1, 1, 1]);
    let cases = [
        (generations([2, 1, 1, 1]), GenerationComponent::Scheduler),
        (generations([1, 2, 1, 1]), GenerationComponent::Backend),
        (generations([1, 1, 2, 1]), GenerationComponent::Safety),
        (
            generations([1, 1, 1, 2]),
            GenerationComponent::RuntimeOverhead,
        ),
    ];

    assert_eq!(planned.validate_current(planned), Ok(()));
    for (current, component) in cases {
        assert_eq!(
            planned.validate_current(current),
            Err(GenerationMismatch {
                component,
                planned: 1,
                current: 2
            })
        );
    }
}

#[test]
fn generations_are_nonzero_and_checked() {
    assert_eq!(SchedulerGeneration::new(0), Err(DomainValueError::Zero));
    let maximum = RuntimeOverheadGeneration::new(u64::MAX).unwrap();
    assert_eq!(maximum.next(), Err(DomainValueError::Overflow));
}

#[test]
fn bounded_collections_reject_capacity_and_duplicates_without_mutation() {
    let mut values = BoundedVec::<u8, 2>::new();
    values.try_push(3).unwrap();
    values.try_push(5).unwrap();
    assert_eq!(values.try_push(8), Err(BoundedCollectionError::Full));
    assert_eq!(values.len(), 2);
    assert_eq!(values.capacity(), 2);
    assert_eq!(values.iter().copied().collect::<Vec<_>>(), vec![3, 5]);
    assert_eq!(
        BoundedVec::<u8, 0>::new().try_push(1),
        Err(BoundedCollectionError::Full)
    );

    let mut set = BoundedSet::<u8, 2>::new();
    set.try_insert(3).unwrap();
    assert_eq!(set.try_insert(3), Err(BoundedCollectionError::Duplicate));
    set.try_insert(5).unwrap();
    assert_eq!(set.try_insert(8), Err(BoundedCollectionError::Full));
    assert!(set.contains(&3));
    assert_eq!(set.iter().copied().collect::<Vec<_>>(), vec![3, 5]);

    let mut map = BoundedMap::<&str, u8, 2>::new();
    map.try_insert("three", 3).unwrap();
    assert_eq!(
        map.try_insert("three", 8),
        Err(BoundedCollectionError::Duplicate)
    );
    assert_eq!(map.get(&"three"), Some(&3));
    map.try_insert("five", 5).unwrap();
    assert_eq!(
        map.try_insert("eight", 8),
        Err(BoundedCollectionError::Full)
    );
    assert_eq!(map.len(), 2);
    assert_eq!(map.get(&"five"), Some(&5));
}
