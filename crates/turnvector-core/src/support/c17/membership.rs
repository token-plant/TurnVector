use super::*;
use crate::SourceRecordRef;
use crate::request_book::c17::SupportMembershipAnchor;

const STANDALONE_BRANCH: u8 = 3;
const STANDALONE_FUNDER_ROWS: usize = PLAN_MEMBERS_MAX;
const STANDALONE_MEMBER_ROWS: usize = PLAN_MEMBERS_MAX;
const STANDALONE_LOCAL_EDITS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MembershipFunding {
    pub(crate) request: RequestId,
    pub(crate) request_key: [u8; 40],
    pub(crate) record_slot: u32,
    pub(crate) owner_header: ArenaRef,
    pub(crate) entitlement: [u8; 32],
    pub(crate) vector: [u8; 32],
    pub(crate) branch_limit: u64,
}

impl MembershipFunding {
    fn validate(self) -> Result<(), SupportLedgerError> {
        if self.request_key == [0; 40]
            || self.owner_header.generation == 0
            || self.owner_header.slot != self.record_slot
            || self.entitlement == [0; 32]
            || self.vector == [0; 32]
            || self.branch_limit == 0
            || crate::request_book::c17::request_key(self.request) != self.request_key
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CreateStandaloneInput {
    pub(crate) authority_key: [u8; 17],
    pub(crate) domain: [u8; 16],
    pub(crate) source: SourceRecordRef,
    pub(crate) initial_kind: u8,
    pub(crate) event_id: u64,
    pub(crate) anchor: SupportMembershipAnchor,
    pub(crate) occurred_at: u64,
    pub(crate) obligation: [u8; 32],
    pub(crate) credit: [u8; 32],
    pub(crate) funding: MembershipFunding,
}

impl CreateStandaloneInput {
    fn validate(self) -> Result<(), SupportLedgerError> {
        self.funding.validate()?;
        if self.authority_key[0] != 0x31
            || self.authority_key[1..] != self.domain
            || self.domain == [0; 16]
            || self.source.is_absent()
            || !matches!(self.initial_kind, 1 | 2)
            || self.event_id == 0
            || self.anchor.is_absent()
            || self.anchor.authority_key() != self.authority_key
            || self.anchor.branch() != STANDALONE_BRANCH
            || self.anchor.group() != self.anchor.root()
            || self.anchor.root_version() != 1
            || self.occurred_at == 0
            || self.obligation == [0; 32]
            || self.credit == [0; 32]
            || self.obligation == self.credit
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        Ok(())
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PreparedCreateStandaloneRoot {
    expected_c17: u64,
    expected_raw: u64,
    expected_authority: u64,
    expected_local: u64,
    expected_arena_headers: [ByteArenaHeaderImage; 10],
    arena_headers_after: [ByteArenaHeaderImage; 10],
    input: CreateStandaloneInput,
    group: ArenaSelection<1>,
    formation: ArenaSelection<1>,
    funders: ArenaSelection<STANDALONE_FUNDER_ROWS>,
    members: ArenaSelection<STANDALONE_MEMBER_ROWS>,
    wrapper: ArenaSelection<1>,
    link: ArenaSelection<1>,
    membership: ArenaSelection<1>,
    mutation: ArenaSelection<1>,
    authority_before: Option<([u8; 8], NodeHandle)>,
    authority_after: [u8; 8],
    raw_before: [[u8; 8]; 2],
    raw_updates: [([u8; 32], NodeHandle, [u8; 8]); 2],
    local_entries: [([u8; 17], [u8; 8]); STANDALONE_LOCAL_EDITS],
    group_image: [u8; GROUP_BYTES],
    formation_image: [u8; FORMATION_BYTES],
    funder_images: [[u8; FUNDER_BYTES]; STANDALONE_FUNDER_ROWS],
    member_images: [[u8; MEMBER_BYTES]; STANDALONE_MEMBER_ROWS],
    wrapper_image: [u8; WRAPPER_BYTES],
    link_image: [u8; LINK_BYTES],
    membership_image: [u8; MEMBERSHIP_BYTES],
    mutation_image: [u8; MUTATION_BYTES],
    owner_references: [ArenaRef; 4],
    owner_record_before: BundleRecord,
    owner_record_after: BundleRecord,
    owner_row_after: [u8; OWNER_ROW_BYTES],
    owner_after: [u8; OWNER_BYTES],
    header_after: C17HeaderImage,
    raw_plan: PatriciaAssignmentPlan<RAW_ASSIGNMENT_MAX>,
    authority_plan: PatriciaAssignmentPlan<AUTHORITY_ASSIGNMENT_MAX>,
    local_plan: PatriciaAssignmentPlan<LOCAL_ASSIGNMENT_MAX>,
}

impl PreparedCreateStandaloneRoot {
    pub(crate) const fn owner_slot(&self) -> u32 {
        self.input.funding.record_slot
    }

    pub(in crate::support) const fn owner_record_before(&self) -> BundleRecord {
        self.owner_record_before
    }

    pub(in crate::support) const fn owner_record_after(&self) -> BundleRecord {
        self.owner_record_after
    }

    pub(crate) const fn anchor(&self) -> SupportMembershipAnchor {
        self.input.anchor
    }

    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.raw_plan.visit_assignments(visitor);
        self.authority_plan.visit_assignments(visitor);
        self.local_plan.visit_assignments(visitor);
    }
}

impl SupportC17 {
    pub(crate) fn preview_create_standalone_anchor(
        &self,
        authority_key: [u8; 17],
    ) -> Result<SupportMembershipAnchor, SupportLedgerError> {
        if authority_key[0] != 0x31 || authority_key[1..] == [0; 16] {
            return Err(SupportLedgerError::InvalidInput);
        }
        let group = self.groups.prepare_reserve::<1>(1)?[0];
        SupportMembershipAnchor::try_new(
            authority_key,
            STANDALONE_BRANCH,
            group.slot,
            group.generation,
            group.slot,
            group.generation,
            1,
        )
        .map_err(|_| SupportLedgerError::InvalidInput)
    }

    pub(in crate::support) fn prepare_create_standalone_root(
        &self,
        input: CreateStandaloneInput,
        owner_record: BundleRecord,
    ) -> Result<PreparedCreateStandaloneRoot, SupportLedgerError> {
        input.validate()?;
        let generation_after = self
            .generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let expected_arena_headers = self.membership_arena_headers();
        if read_u32(&self.header.0, 88) >= CREATE_STANDALONE_BUDGET as u32 {
            return Err(capacity_error());
        }

        let group = self.groups.prepare_reserve::<1>(1)?;
        let formation = self.formations.prepare_reserve::<1>(1)?;
        let funders = self
            .funders
            .prepare_reserve::<STANDALONE_FUNDER_ROWS>(STANDALONE_FUNDER_ROWS)?;
        let members = self
            .members
            .prepare_reserve::<STANDALONE_MEMBER_ROWS>(STANDALONE_MEMBER_ROWS)?;
        let wrapper = self.wrappers.prepare_reserve::<1>(1)?;
        let link = self.links.prepare_reserve::<1>(1)?;
        let membership = self.memberships.prepare_reserve::<1>(1)?;
        let mutation = self.mutations.prepare_reserve::<1>(1)?;
        let expected_anchor = SupportMembershipAnchor::try_new(
            input.authority_key,
            STANDALONE_BRANCH,
            group[0].slot,
            group[0].generation,
            group[0].slot,
            group[0].generation,
            1,
        )
        .map_err(|_| SupportLedgerError::InvalidInput)?;
        if input.anchor != expected_anchor {
            return Err(SupportLedgerError::Generation);
        }

        let authority_after = encode_arena_ref_value(group[0]);
        let authority_before = if let Some(before) = self.authority.find(&input.authority_key)? {
            let handle = self
                .authority
                .find_handle(&input.authority_key)?
                .ok_or_else(noncanonical_error)?;
            let previous = decode_arena_ref(&before)?;
            let previous_group = self.groups.image(previous, &[1])?;
            if previous_group[40..57] != input.authority_key {
                return Err(noncanonical_error());
            }
            self.authority.validate_update_batch(&[(
                input.authority_key,
                handle,
                authority_after,
            )])?;
            Some((before, handle))
        } else {
            self.authority
                .validate_insert_batch(&[(input.authority_key, authority_after)])?;
            None
        };
        let initial_ordinal = usize::from(input.initial_kind - 1);
        let requirement = owner_record.initial[initial_ordinal];
        if requirement.obligation.get() != input.obligation
            || requirement.credit.get() != input.credit
            || requirement.state != SupportObligationState::Retained
        {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let raw_specs = [
            (
                input.obligation,
                initial_ordinal,
                encode_raw_owner_at(
                    RawOwnerKind::PlanRoot,
                    RawOwnerState::Committed,
                    STANDALONE_BRANCH,
                    wrapper[0],
                )?,
            ),
            (
                input.credit,
                initial_ordinal + 3,
                encode_raw_owner_at(
                    RawOwnerKind::Formation,
                    RawOwnerState::Committed,
                    STANDALONE_BRANCH,
                    wrapper[0],
                )?,
            ),
        ];
        let mut raw_edits = [([0; 32], NodeHandle::SENTINEL, [0; 8], [0; 8]); 2];
        for (index, (key, ordinal, after)) in raw_specs.into_iter().enumerate() {
            let before = self.raw.find(&key)?.ok_or_else(noncanonical_error)?;
            let handle = self.raw.find_handle(&key)?.ok_or_else(noncanonical_error)?;
            let (kind, state, stored_ordinal, owner) = decode_raw_owner_at(before)?;
            if kind != c16_raw_kind(ordinal)?
                || state != RawOwnerState::Committed
                || usize::from(stored_ordinal) != ordinal
                || owner != input.funding.owner_header
            {
                return Err(noncanonical_error());
            }
            raw_edits[index] = (key, handle, before, after);
        }
        raw_edits.sort_unstable_by_key(|entry| entry.0);
        if raw_edits[0].0 >= raw_edits[1].0 {
            return Err(SupportLedgerError::InvalidInput);
        }
        let raw_before = raw_edits.map(|entry| entry.2);
        let raw_updates = raw_edits.map(|entry| (entry.0, entry.1, entry.3));
        self.raw.validate_update_batch(&raw_updates)?;

        let mut local_entries = [([0; 17], [0; 8]); STANDALONE_LOCAL_EDITS];
        let mut local_count = 0;
        for (kind, reference) in [
            (LocalKind::Group, group[0]),
            (LocalKind::Funder, funders[0]),
            (LocalKind::Funder, funders[1]),
            (LocalKind::Funder, funders[2]),
            (LocalKind::Funder, funders[3]),
            (LocalKind::Link, link[0]),
            (LocalKind::Membership, membership[0]),
            (LocalKind::Mutation, mutation[0]),
        ] {
            local_entries[local_count] = (
                local_key(kind, reference),
                encode_arena_ref_value(reference),
            );
            local_count += 1;
        }
        debug_assert_eq!(local_count, STANDALONE_LOCAL_EDITS);
        local_entries.sort_unstable_by_key(|entry| entry.0);
        if local_entries.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err(SupportLedgerError::InvalidInput);
        }
        self.local.validate_insert_batch(&local_entries)?;

        let funding = input.funding;
        let owner_references = [
            self.owner_headers.reference_at(funding.record_slot, &[1])?,
            self.owner_rows.reference_at(funding.record_slot, &[1])?,
            self.owner_indices.reference_at(funding.record_slot, &[1])?,
            self.owners.reference_at(funding.record_slot, &[1])?,
        ];
        if owner_references[0] != funding.owner_header
            || owner_record.request_owner != funding.request
            || owner_record.entitlement.get() != funding.entitlement
            || owner_record.vector.get() != funding.vector
            || !matches!(
                owner_record.state,
                BundleState::LivePristine | BundleState::LiveConsumed
            )
        {
            return Err(SupportLedgerError::InvalidTransition);
        }
        validate_c16_owner_set(
            [
                self.owner_headers
                    .image(owner_references[0], &[1])?
                    .as_slice(),
                self.owner_rows.image(owner_references[1], &[1])?.as_slice(),
                self.owner_indices
                    .image(owner_references[2], &[1])?
                    .as_slice(),
                self.owners.image(owner_references[3], &[1])?.as_slice(),
            ],
            owner_references,
            funding.record_slot,
            &owner_record,
            OWNER_STATE_LIVE,
        )?;
        let mut owner_row_after = *self.owner_rows.image(owner_references[1], &[1])?;
        let mut owner_after = *self.owners.image(owner_references[3], &[1])?;
        if decode_optional_arena_ref(
            &owner_row_after[OWNER_ROW_ACTIVE_LINK..OWNER_ROW_ACTIVE_LINK + 8],
        )?
        .is_some()
        {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let linked = owner_record
            .linked_claims
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let current = read_u64(&owner_row_after, OWNER_ROW_CURRENT)
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let branch_current = read_u64(
            &owner_row_after,
            OWNER_ROW_BRANCH_CURRENT + usize::from(STANDALONE_BRANCH) * 8,
        )
        .checked_add(1)
        .ok_or_else(capacity_error)?;
        if branch_current > funding.branch_limit {
            return Err(capacity_error());
        }
        write_u32(&mut owner_row_after, OWNER_ROW_LINKED_CLAIMS, linked);
        write_u64(&mut owner_row_after, OWNER_ROW_CURRENT, current);
        write_u64(
            &mut owner_row_after,
            OWNER_ROW_BRANCH_CURRENT + usize::from(STANDALONE_BRANCH) * 8,
            branch_current,
        );
        encode_arena_ref(
            &mut owner_row_after[OWNER_ROW_ACTIVE_LINK..OWNER_ROW_ACTIVE_LINK + 8],
            link[0],
        );
        write_u32(&mut owner_after, OWNER_IMAGE_LINKED_CLAIMS, linked);
        let mut owner_record_after = owner_record;
        owner_record_after.linked_claims = linked;
        if owner_record_after.state == BundleState::LivePristine {
            owner_record_after.state = BundleState::LiveConsumed;
        }

        let group_image = self.groups.prepare_reserved_image_after(
            group[0],
            encode_membership_group(
                STANDALONE_BRANCH,
                RootState::Conditional,
                2,
                input.authority_key,
                formation[0],
                wrapper[0],
                members
                    .as_slice()
                    .try_into()
                    .expect("four standalone members"),
                1,
            ),
            1,
        )?;
        let formation_image = self.formations.prepare_reserved_image_after(
            formation[0],
            encode_standalone_formation(input, group[0], wrapper[0]),
            1,
        )?;
        let wrapper_image = self.wrappers.prepare_reserved_image_after(
            wrapper[0],
            encode_membership_wrapper(
                STANDALONE_BRANCH,
                RootState::Conditional,
                group[0],
                formation[0],
                input.authority_key,
                1,
            ),
            1,
        )?;
        let mut funder_images = [[0; FUNDER_BYTES]; STANDALONE_FUNDER_ROWS];
        let mut member_images = [[0; MEMBER_BYTES]; STANDALONE_MEMBER_ROWS];
        for ordinal in 0..PLAN_MEMBERS_MAX {
            let active = ordinal == 0;
            funder_images[ordinal] = self.funders.prepare_reserved_image_after(
                funders[ordinal],
                encode_membership_funder(
                    STANDALONE_BRANCH,
                    ordinal,
                    active,
                    group[0],
                    formation[0],
                    members[ordinal],
                    funding,
                    1,
                ),
                1,
            )?;
            member_images[ordinal] = self.members.prepare_reserved_image_after(
                members[ordinal],
                encode_membership_member(
                    STANDALONE_BRANCH,
                    ordinal,
                    active,
                    group[0],
                    funders[ordinal],
                    funding,
                ),
                1,
            )?;
        }
        let link_image = self.links.prepare_reserved_image_after(
            link[0],
            encode_plan_link(
                funding.owner_header,
                group[0],
                formation[0],
                input.authority_key,
                generation_after,
            ),
            1,
        )?;
        let membership_image = self.memberships.prepare_reserved_image_after(
            membership[0],
            encode_membership_event_image(
                SemanticOperation::CreateStandalone,
                input.event_id,
                input.source,
                input.anchor,
                funding.request_key,
                generation_after,
                input.occurred_at,
            ),
            1,
        )?;
        let mutation_image = self.mutations.prepare_reserved_image_after(
            mutation[0],
            encode_membership_mutation(
                SemanticOperation::CreateStandalone,
                input.event_id,
                group[0],
                formation[0],
                generation_after,
                input.occurred_at,
            ),
            1,
        )?;
        self.owner_rows.validate_advance_generation()?;
        self.owners.validate_advance_generation()?;
        let arena_headers_after = self.prepare_membership_arena_headers_after(
            &group,
            &formation,
            &funders,
            &members,
            &wrapper,
            &link,
            &membership,
            &mutation,
        )?;
        let raw_plan = self
            .raw
            .prepare_update_assignment_plan(RAW_INDEX_ASSIGNMENT_ARENA, &raw_updates)?;
        let authority_plan = match authority_before {
            Some((_, handle)) => self.authority.prepare_update_assignment_plan(
                AUTHORITY_INDEX_ASSIGNMENT_ARENA,
                &[(input.authority_key, handle, authority_after)],
            )?,
            None => self.authority.prepare_insert_assignment_plan(
                AUTHORITY_INDEX_ASSIGNMENT_ARENA,
                &[(input.authority_key, authority_after)],
            )?,
        };
        let local_plan = self
            .local
            .prepare_insert_assignment_plan(LOCAL_INDEX_ASSIGNMENT_ARENA, &local_entries)?;
        let next_create_count = read_u32(&self.header.0, 88)
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let mut header_after = self.header;
        write_u32(&mut header_after.0, 88, next_create_count);
        write_u64(&mut header_after.0, 48, generation_after);

        Ok(PreparedCreateStandaloneRoot {
            expected_c17: self.generation(),
            expected_raw: self.raw.generation(),
            expected_authority: self.authority.generation(),
            expected_local: self.local.generation(),
            expected_arena_headers,
            arena_headers_after,
            input,
            group,
            formation,
            funders,
            members,
            wrapper,
            link,
            membership,
            mutation,
            authority_before,
            authority_after,
            raw_before,
            raw_updates,
            local_entries,
            group_image,
            formation_image,
            funder_images,
            member_images,
            wrapper_image,
            link_image,
            membership_image,
            mutation_image,
            owner_references,
            owner_record_before: owner_record,
            owner_record_after,
            owner_row_after,
            owner_after,
            header_after,
            raw_plan,
            authority_plan,
            local_plan,
        })
    }

    pub(in crate::support) fn validate_create_standalone_root(
        &self,
        change: &PreparedCreateStandaloneRoot,
        owner_record: BundleRecord,
    ) -> Result<(), SupportLedgerError> {
        change.input.validate()?;
        let generation_after = self
            .generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let next_create_count = read_u32(&self.header.0, 88)
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let mut header_after = self.header;
        write_u32(&mut header_after.0, 88, next_create_count);
        write_u64(&mut header_after.0, 48, generation_after);
        let expected_arena_headers = self.membership_arena_headers();
        let arena_headers_after = self.prepare_membership_arena_headers_after(
            &change.group,
            &change.formation,
            &change.funders,
            &change.members,
            &change.wrapper,
            &change.link,
            &change.membership,
            &change.mutation,
        )?;
        if self.generation() != change.expected_c17
            || self.raw.generation() != change.expected_raw
            || self.authority.generation() != change.expected_authority
            || self.local.generation() != change.expected_local
            || expected_arena_headers != change.expected_arena_headers
            || arena_headers_after != change.arena_headers_after
            || header_after != change.header_after
            || owner_record != change.owner_record_before
            || self.groups.prepare_reserve::<1>(1)?.as_slice() != change.group.as_slice()
            || self.formations.prepare_reserve::<1>(1)?.as_slice() != change.formation.as_slice()
            || self
                .funders
                .prepare_reserve::<STANDALONE_FUNDER_ROWS>(STANDALONE_FUNDER_ROWS)?
                .as_slice()
                != change.funders.as_slice()
            || self
                .members
                .prepare_reserve::<STANDALONE_MEMBER_ROWS>(STANDALONE_MEMBER_ROWS)?
                .as_slice()
                != change.members.as_slice()
            || self.wrappers.prepare_reserve::<1>(1)?.as_slice() != change.wrapper.as_slice()
            || self.links.prepare_reserve::<1>(1)?.as_slice() != change.link.as_slice()
            || self.memberships.prepare_reserve::<1>(1)?.as_slice() != change.membership.as_slice()
            || self.mutations.prepare_reserve::<1>(1)?.as_slice() != change.mutation.as_slice()
            || self.authority.find(&change.input.authority_key)?
                != change.authority_before.map(|(value, _)| value)
            || read_u32(&self.header.0, 88) >= CREATE_STANDALONE_BUDGET as u32
            || !self.raw.validates_assignment_plan(&change.raw_plan)
            || !self
                .authority
                .validates_assignment_plan(&change.authority_plan)
            || !self.local.validates_assignment_plan(&change.local_plan)
        {
            return Err(SupportLedgerError::Generation);
        }
        validate_c16_owner_set(
            [
                self.owner_headers
                    .image(change.owner_references[0], &[1])?
                    .as_slice(),
                self.owner_rows
                    .image(change.owner_references[1], &[1])?
                    .as_slice(),
                self.owner_indices
                    .image(change.owner_references[2], &[1])?
                    .as_slice(),
                self.owners
                    .image(change.owner_references[3], &[1])?
                    .as_slice(),
            ],
            change.owner_references,
            change.input.funding.record_slot,
            &owner_record,
            OWNER_STATE_LIVE,
        )?;
        match change.authority_before {
            Some((_, handle)) => self.authority.validate_update_batch(&[(
                change.input.authority_key,
                handle,
                change.authority_after,
            )])?,
            None => self
                .authority
                .validate_insert_batch(&[(change.input.authority_key, change.authority_after)])?,
        }
        for (index, (key, _, _)) in change.raw_updates.iter().enumerate() {
            if self.raw.find(key)? != Some(change.raw_before[index]) {
                return Err(SupportLedgerError::Generation);
            }
        }
        self.raw.validate_update_batch(&change.raw_updates)?;
        self.local.validate_insert_batch(&change.local_entries)?;
        self.owner_rows.validate_advance_generation()?;
        self.owners.validate_advance_generation()?;
        let reconstructed = self.prepare_create_standalone_root(change.input, owner_record)?;
        if &reconstructed != change {
            return Err(SupportLedgerError::Generation);
        }
        Ok(())
    }

    pub(in crate::support) fn commit_create_standalone_root(
        &mut self,
        change: PreparedCreateStandaloneRoot,
        owner_record: BundleRecord,
    ) {
        self.validate_create_standalone_root(&change, owner_record)
            .expect("validated CreateStandalone Support root");
        self.commit_create_standalone_root_prevalidated(change, true);
    }

    pub(in crate::support) fn commit_create_standalone_root_prevalidated(
        &mut self,
        change: PreparedCreateStandaloneRoot,
        apply_index_plans: bool,
    ) {
        self.groups
            .install_reserved_image_direct(change.group[0], change.group_image);
        self.formations
            .install_reserved_image_direct(change.formation[0], change.formation_image);
        for ordinal in 0..PLAN_MEMBERS_MAX {
            self.funders.install_reserved_image_direct(
                change.funders[ordinal],
                change.funder_images[ordinal],
            );
            self.members.install_reserved_image_direct(
                change.members[ordinal],
                change.member_images[ordinal],
            );
        }
        self.wrappers
            .install_reserved_image_direct(change.wrapper[0], change.wrapper_image);
        self.links
            .install_reserved_image_direct(change.link[0], change.link_image);
        self.memberships
            .install_reserved_image_direct(change.membership[0], change.membership_image);
        self.mutations
            .install_reserved_image_direct(change.mutation[0], change.mutation_image);
        self.owner_rows
            .replace_image_prevalidated(change.owner_references[1], change.owner_row_after);
        self.owners
            .replace_image_prevalidated(change.owner_references[3], change.owner_after);
        if apply_index_plans {
            self.authority
                .commit_assignment_plan_prevalidated(change.authority_plan);
            self.raw
                .commit_assignment_plan_prevalidated(change.raw_plan);
            self.local
                .commit_assignment_plan_prevalidated(change.local_plan);
        }
        self.assign_membership_arena_headers(change.arena_headers_after);
        self.header = change.header_after;
    }

    fn prepare_membership_arena_headers_after(
        &self,
        group: &ArenaSelection<1>,
        formation: &ArenaSelection<1>,
        funders: &ArenaSelection<STANDALONE_FUNDER_ROWS>,
        members: &ArenaSelection<STANDALONE_MEMBER_ROWS>,
        wrapper: &ArenaSelection<1>,
        link: &ArenaSelection<1>,
        membership: &ArenaSelection<1>,
        mutation: &ArenaSelection<1>,
    ) -> Result<[ByteArenaHeaderImage; 10], SupportLedgerError> {
        Ok([
            self.groups
                .prepare_reserve_header_after(group, group.len(), 0)?,
            self.formations
                .prepare_reserve_header_after(formation, formation.len(), 0)?,
            self.funders
                .prepare_reserve_header_after(funders, funders.len(), 0)?,
            self.members
                .prepare_reserve_header_after(members, members.len(), 0)?,
            self.wrappers
                .prepare_reserve_header_after(wrapper, wrapper.len(), 0)?,
            self.owner_rows.prepare_generation_header_after()?,
            self.owners.prepare_generation_header_after()?,
            self.links
                .prepare_reserve_header_after(link, link.len(), 0)?,
            self.memberships
                .prepare_reserve_header_after(membership, membership.len(), 0)?,
            self.mutations
                .prepare_reserve_header_after(mutation, mutation.len(), 0)?,
        ])
    }

    fn assign_membership_arena_headers(&mut self, headers: [ByteArenaHeaderImage; 10]) {
        self.groups.assign_header_direct(headers[0]);
        self.formations.assign_header_direct(headers[1]);
        self.funders.assign_header_direct(headers[2]);
        self.members.assign_header_direct(headers[3]);
        self.wrappers.assign_header_direct(headers[4]);
        self.owner_rows.assign_header_direct(headers[5]);
        self.owners.assign_header_direct(headers[6]);
        self.links.assign_header_direct(headers[7]);
        self.memberships.assign_header_direct(headers[8]);
        self.mutations.assign_header_direct(headers[9]);
    }

    fn membership_arena_headers(&self) -> [ByteArenaHeaderImage; 10] {
        [
            self.groups.header_image(),
            self.formations.header_image(),
            self.funders.header_image(),
            self.members.header_image(),
            self.wrappers.header_image(),
            self.owner_rows.header_image(),
            self.owners.header_image(),
            self.links.header_image(),
            self.memberships.header_image(),
            self.mutations.header_image(),
        ]
    }
}

pub(super) fn encode_membership_group(
    branch: u8,
    state: RootState,
    locator_kind: u8,
    authority_key: [u8; 17],
    formation: ArenaRef,
    locator: ArenaRef,
    members: [ArenaRef; PLAN_MEMBERS_MAX],
    member_count: usize,
) -> [u8; GROUP_BYTES] {
    let mut image = [0; GROUP_BYTES];
    image[8] = branch;
    image[9] = state as u8;
    image[10] = member_count as u8;
    image[11] = locator_kind;
    encode_arena_ref(&mut image[16..24], formation);
    encode_arena_ref(&mut image[24..32], locator);
    write_u64(&mut image, 32, 1);
    image[40..57].copy_from_slice(&authority_key);
    for (ordinal, reference) in members.into_iter().enumerate() {
        encode_arena_ref(&mut image[64 + ordinal * 8..72 + ordinal * 8], reference);
    }
    image
}

fn encode_standalone_formation(
    input: CreateStandaloneInput,
    group: ArenaRef,
    wrapper: ArenaRef,
) -> [u8; FORMATION_BYTES] {
    let mut image = [0; FORMATION_BYTES];
    image[8..16].copy_from_slice(&encode_source_ref_word(input.source));
    write_u64(&mut image, 16, input.event_id);
    image[24..40].copy_from_slice(&input.domain);
    image[40..72].copy_from_slice(&input.obligation);
    image[72..104].copy_from_slice(&input.credit);
    image[104..121].copy_from_slice(&input.authority_key);
    image[220] = STANDALONE_BRANCH;
    image[221] = RootState::Conditional as u8;
    image[222] = FormationCause::InitialReady as u8;
    write_u64(&mut image, 224, 1);
    write_u64(&mut image, 232, input.occurred_at);
    encode_arena_ref(&mut image[240..248], group);
    encode_arena_ref(&mut image[248..256], wrapper);
    image
}

pub(super) fn encode_membership_wrapper(
    branch: u8,
    state: RootState,
    group: ArenaRef,
    formation: ArenaRef,
    authority_key: [u8; 17],
    version: u64,
) -> [u8; WRAPPER_BYTES] {
    let mut image = [0; WRAPPER_BYTES];
    image[8] = branch;
    image[9] = state as u8;
    encode_arena_ref(&mut image[16..24], group);
    encode_arena_ref(&mut image[24..32], formation);
    image[32..49].copy_from_slice(&authority_key);
    write_u64(&mut image, 56, version);
    image
}

pub(super) fn encode_membership_funder(
    branch: u8,
    ordinal: usize,
    active: bool,
    group: ArenaRef,
    formation: ArenaRef,
    member: ArenaRef,
    funding: MembershipFunding,
    version: u8,
) -> [u8; FUNDER_BYTES] {
    let mut image = [0; FUNDER_BYTES];
    image[8] = u8::from(active);
    image[9] = branch;
    image[10] = version;
    image[11] = ordinal as u8;
    encode_arena_ref(&mut image[16..24], group);
    encode_arena_ref(&mut image[24..32], formation);
    encode_arena_ref(&mut image[32..40], member);
    if active {
        encode_arena_ref(&mut image[40..48], funding.owner_header);
        image[48..80].copy_from_slice(&funding.entitlement);
        image[80..112].copy_from_slice(&funding.vector);
        write_u64(&mut image, 112, 1);
        write_u64(&mut image, 120, funding.branch_limit);
    }
    image
}

pub(super) fn encode_membership_member(
    branch: u8,
    ordinal: usize,
    active: bool,
    group: ArenaRef,
    funder: ArenaRef,
    funding: MembershipFunding,
) -> [u8; MEMBER_BYTES] {
    let mut image = [0; MEMBER_BYTES];
    image[8] = u8::from(active);
    image[9] = branch;
    image[10] = ordinal as u8;
    encode_arena_ref(&mut image[16..24], group);
    encode_arena_ref(&mut image[24..32], funder);
    if active {
        image[32..72].copy_from_slice(&funding.request_key);
        encode_arena_ref(&mut image[72..80], funding.owner_header);
        image[80..112].copy_from_slice(&funding.entitlement);
    }
    image
}

pub(super) fn encode_membership_event_image(
    operation: SemanticOperation,
    event_id: u64,
    source: SourceRecordRef,
    anchor: SupportMembershipAnchor,
    request_key: [u8; 40],
    generation: u64,
    occurred_at: u64,
) -> [u8; MEMBERSHIP_BYTES] {
    let mut image = [0; MEMBERSHIP_BYTES];
    image[8] = operation as u8;
    image[9] = 1;
    write_u64(&mut image, 16, event_id);
    image[24..32].copy_from_slice(&encode_source_ref_word(source));
    image[32..72].copy_from_slice(&request_key);
    image[72..104].copy_from_slice(&anchor.bytes()[..32]);
    write_u64(&mut image, 104, generation ^ occurred_at);
    image
}

pub(super) fn encode_membership_mutation(
    operation: SemanticOperation,
    event_id: u64,
    group: ArenaRef,
    formation: ArenaRef,
    generation: u64,
    occurred_at: u64,
) -> [u8; MUTATION_BYTES] {
    let mut image = [0; MUTATION_BYTES];
    image[8] = operation as u8;
    image[9] = 1;
    write_u64(&mut image, 16, generation);
    write_u64(&mut image, 24, occurred_at);
    write_u64(&mut image, 32, event_id);
    encode_arena_ref(&mut image[40..48], group);
    encode_arena_ref(&mut image[48..56], formation);
    image
}

fn encode_source_ref_word(reference: SourceRecordRef) -> [u8; 8] {
    let mut bytes = [0; 8];
    bytes[..2].copy_from_slice(&reference.slot().to_le_bytes());
    bytes[4..].copy_from_slice(&reference.generation().to_le_bytes());
    bytes
}
