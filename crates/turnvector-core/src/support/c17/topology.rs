use super::semantic::{
    AggregateDelta, RootMemberSnapshot, RootSnapshot, apply_i32_u32, apply_i32_u64,
    request_id_from_key_for_support, transition_aggregate,
};
use super::*;
use crate::request_book::c17::{
    MembershipDestination, MembershipEventKind, MembershipEventRecord, MembershipTag,
    PreparedCancellation, PreparedMembershipIntent, SupportMembershipAnchor,
};
use crate::work::WorkRecorder;

const SOURCE_MAX: usize = 3;
const FORMATION_MAX: usize = SOURCE_MAX + 1;
const FUNDER_MAX: usize = FORMATION_MAX * PLAN_MEMBERS_MAX;
const MEMBER_MAX: usize = PLAN_MEMBERS_MAX;
const WRAPPER_MAX: usize = SOURCE_MAX + 1;
const LINK_MAX: usize = PLAN_MEMBERS_MAX;
const MUTATION_MAX: usize = SOURCE_MAX + 1;
const LOCAL_MAX: usize = 32;
const STANDALONE_BRANCH: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TopologyOwner {
    slot: u32,
    owner: ArenaRef,
    request_key: [u8; 40],
    source: usize,
    branch_delta: [i32; 4],
    vector_delta: [i32; 4],
    linked_delta: i32,
}

impl TopologyOwner {
    const ZERO: Self = Self {
        slot: 0,
        owner: ArenaRef {
            slot: 0,
            generation: 0,
        },
        request_key: [0; 40],
        source: 0,
        branch_delta: [0; 4],
        vector_delta: [0; 4],
        linked_delta: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MergeInitialPreview {
    expected_c17: u64,
    sources: [Option<RootSnapshot>; SOURCE_MAX],
    source_count: usize,
    destination: SupportMembershipAnchor,
    aggregate: AggregateDelta,
    owners: [TopologyOwner; PLAN_MEMBERS_MAX],
    owner_count: usize,
    occurred_at: u64,
}

impl MergeInitialPreview {
    pub(crate) const fn destination(&self) -> SupportMembershipAnchor {
        self.destination
    }

    pub(crate) const fn aggregate_delta(&self) -> AggregateDelta {
        self.aggregate
    }

    pub(crate) const fn owner_count(&self) -> usize {
        self.owner_count
    }

    pub(crate) fn owner_slots(&self) -> [u32; PLAN_MEMBERS_MAX] {
        let mut slots = [0; PLAN_MEMBERS_MAX];
        let mut index = 0;
        while index < self.owner_count {
            slots[index] = self.owners[index].slot;
            index += 1;
        }
        slots
    }

    pub(crate) fn owner_branch_delta(&self, index: usize) -> Option<[i32; 4]> {
        (index < self.owner_count).then_some(self.owners[index].vector_delta)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SourceJournal {
    before: RootSnapshot,
    locator_after_ref: ArenaRef,
    group_after: [u8; GROUP_BYTES],
    locator_after: [u8; WRAPPER_BYTES],
    formation_after: [u8; FORMATION_BYTES],
    funder_after: [[u8; FUNDER_BYTES]; PLAN_MEMBERS_MAX],
    member_after: [[u8; MEMBER_BYTES]; PLAN_MEMBERS_MAX],
    mutation_after: [u8; MUTATION_BYTES],
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PreparedMergeInitialTopology {
    expected_c17: u64,
    expected_authority: u64,
    expected_local: u64,
    expected_arena_headers: [ByteArenaHeaderImage; 11],
    arena_headers_after: [ByteArenaHeaderImage; 11],
    preview: MergeInitialPreview,
    event: MembershipEventRecord,
    groups: ArenaSelection<1>,
    formations: ArenaSelection<FORMATION_MAX>,
    funders: ArenaSelection<FUNDER_MAX>,
    members: ArenaSelection<MEMBER_MAX>,
    wrappers: ArenaSelection<WRAPPER_MAX>,
    links: ArenaSelection<LINK_MAX>,
    memberships: ArenaSelection<1>,
    mutations: ArenaSelection<MUTATION_MAX>,
    source_journals: [Option<SourceJournal>; SOURCE_MAX],
    destination_group: [u8; GROUP_BYTES],
    destination_formation: [u8; FORMATION_BYTES],
    destination_funders: [[u8; FUNDER_BYTES]; PLAN_MEMBERS_MAX],
    destination_members: [[u8; MEMBER_BYTES]; PLAN_MEMBERS_MAX],
    destination_wrapper: [u8; WRAPPER_BYTES],
    membership_image: [u8; MEMBERSHIP_BYTES],
    membership_mutation: [u8; MUTATION_BYTES],
    local_entries: [([u8; 17], [u8; 8]); LOCAL_MAX],
    local_count: usize,
    authority_key: [u8; 17],
    authority_before: [u8; 8],
    authority_handle: NodeHandle,
    authority_after: [u8; 8],
    owner_records_before: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
    owner_records_after: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
    owner_references: [[ArenaRef; 4]; PLAN_MEMBERS_MAX],
    owner_rows_after: [[u8; OWNER_ROW_BYTES]; PLAN_MEMBERS_MAX],
    owners_after: [[u8; OWNER_BYTES]; PLAN_MEMBERS_MAX],
    retired_links: [ArenaRef; PLAN_MEMBERS_MAX],
    retired_link_before: [[u8; LINK_BYTES]; PLAN_MEMBERS_MAX],
    retired_link_after: [[u8; LINK_BYTES]; PLAN_MEMBERS_MAX],
    replacement_links: [[u8; LINK_BYTES]; PLAN_MEMBERS_MAX],
    header_after: C17HeaderImage,
    authority_plan: PatriciaAssignmentPlan<AUTHORITY_ASSIGNMENT_MAX>,
    local_plan: PatriciaAssignmentPlan<LOCAL_ASSIGNMENT_MAX>,
}

impl PreparedMergeInitialTopology {
    pub(crate) const fn aggregate_delta(&self) -> AggregateDelta {
        self.preview.aggregate
    }

    pub(crate) const fn owner_count(&self) -> usize {
        self.preview.owner_count
    }

    pub(crate) fn owner_slots(&self) -> [u32; PLAN_MEMBERS_MAX] {
        self.preview.owner_slots()
    }

    pub(crate) fn owner_branch_delta(&self, index: usize) -> Option<[i32; 4]> {
        self.preview.owner_branch_delta(index)
    }

    pub(in crate::support) const fn owner_records_after(
        &self,
    ) -> [Option<BundleRecord>; PLAN_MEMBERS_MAX] {
        self.owner_records_after
    }

    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.authority_plan.visit_assignments(visitor);
        self.local_plan.visit_assignments(visitor);
    }
}

impl SupportC17 {
    pub(crate) fn inspect_merge_initial(
        &self,
        anchors: [SupportMembershipAnchor; SOURCE_MAX],
        source_count: u8,
        domain: [u8; 16],
        occurred_at: u64,
    ) -> Result<MergeInitialPreview, SupportLedgerError> {
        let count = usize::from(source_count);
        if !(2..=SOURCE_MAX).contains(&count)
            || domain == [0; 16]
            || occurred_at == 0
            || anchors[..count].iter().any(|anchor| anchor.is_absent())
            || anchors[count..].iter().any(|anchor| !anchor.is_absent())
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        let mut authority_key = [0; 17];
        authority_key[0] = 0x31;
        authority_key[1..].copy_from_slice(&domain);
        let mut roots: [Option<RootSnapshot>; SOURCE_MAX] = [None; SOURCE_MAX];
        for index in 0..count {
            let anchor = anchors[index];
            if anchor.authority_key() != authority_key
                || anchor.branch() != STANDALONE_BRANCH
                || anchor.group() != anchor.root()
                || anchors[..index].contains(&anchor)
            {
                return Err(SupportLedgerError::InvalidTransition);
            }
            let root = self.root_at_group(anchor.group(), authority_key, STANDALONE_BRANCH)?;
            if root.state != RootState::Pending
                || root.member_count != 1
                || root.locator_kind != 2
                || occurred_at <= root.occurred_at
                || roots[..index]
                    .iter()
                    .flatten()
                    .any(|prior| prior.group == root.group)
            {
                return Err(SupportLedgerError::InvalidTransition);
            }
            roots[index] = Some(root);
        }
        roots[..count].sort_unstable_by_key(|root| {
            root.expect("active MergeInitial source").members[0].request_key
        });
        let mut aggregate = AggregateDelta::ZERO;
        let mut owners = [TopologyOwner::ZERO; PLAN_MEMBERS_MAX];
        for index in 0..count {
            let root = roots[index].expect("active MergeInitial source");
            let member = root.members[0];
            if index > 0 {
                let previous = owners[index - 1];
                if previous.request_key >= member.request_key
                    || previous.owner == member.owner
                    || roots[..index].iter().flatten().any(|prior| {
                        let prior = prior.members[0];
                        prior.entitlement == member.entitlement || prior.vector == member.vector
                    })
                {
                    return Err(SupportLedgerError::InvalidInput);
                }
            }
            aggregate.add(transition_aggregate(
                RootState::Pending,
                RootState::ClosedPending,
                1,
            )?)?;
            owners[index] = TopologyOwner {
                slot: member.owner.slot,
                owner: member.owner,
                request_key: member.request_key,
                source: index,
                branch_delta: [0; 4],
                vector_delta: [0; 4],
                linked_delta: 0,
            };
        }
        aggregate.add(materialize_pending_delta(count)?)?;
        let destination_group = self.groups.prepare_reserve::<1>(1)?[0];
        let destination = SupportMembershipAnchor::try_new(
            authority_key,
            STANDALONE_BRANCH,
            destination_group.slot,
            destination_group.generation,
            destination_group.slot,
            destination_group.generation,
            1,
        )
        .map_err(|_| SupportLedgerError::InvalidTransition)?;
        Ok(MergeInitialPreview {
            expected_c17: self.generation(),
            sources: roots,
            source_count: count,
            destination,
            aggregate,
            owners,
            owner_count: count,
            occurred_at,
        })
    }

    pub(in crate::support) fn prepare_merge_initial_topology(
        &self,
        preview: MergeInitialPreview,
        event: MembershipEventRecord,
        owner_records: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
        work: &mut impl WorkRecorder,
    ) -> Result<PreparedMergeInitialTopology, SupportLedgerError> {
        self.validate_merge_initial_preview(&preview)?;
        validate_merge_initial_event(&preview, &event)?;
        let generation_after = self
            .generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let expected_arena_headers = self.topology_arena_headers();
        if read_u32(&self.header.0, 92) >= MERGE_INITIAL_BUDGET as u32 {
            return Err(capacity_error());
        }
        let count = preview.source_count;
        let groups = self.groups.prepare_reserve::<1>(1)?;
        if groups[0] != preview.destination.group() {
            return Err(SupportLedgerError::Generation);
        }
        let formations = self
            .formations
            .prepare_reserve::<FORMATION_MAX>(count + 1)?;
        let funders = self
            .funders
            .prepare_reserve::<FUNDER_MAX>((count + 1) * PLAN_MEMBERS_MAX)?;
        let members = self
            .members
            .prepare_reserve::<MEMBER_MAX>(PLAN_MEMBERS_MAX)?;
        let wrappers = self.wrappers.prepare_reserve::<WRAPPER_MAX>(count + 1)?;
        let links = self
            .links
            .prepare_reserve::<LINK_MAX>(preview.owner_count)?;
        let memberships = self.memberships.prepare_reserve::<1>(1)?;
        let mutations = self.mutations.prepare_reserve::<MUTATION_MAX>(count + 1)?;
        self.owner_rows.validate_advance_generation()?;
        self.owners.validate_advance_generation()?;

        let mut source_journals = [None; SOURCE_MAX];
        let mut local_entries = [([0; 17], [0; 8]); LOCAL_MAX];
        let mut local_count = 0usize;
        for index in 0..count {
            let before = preview.sources[index].expect("active MergeInitial source");
            let formation = formations[index];
            let wrapper = wrappers[index];
            let mut group_after = before.group_image;
            group_after[9] = RootState::ClosedPending as u8;
            encode_arena_ref(&mut group_after[16..24], formation);
            encode_arena_ref(&mut group_after[24..32], wrapper);
            write_u64(&mut group_after, 32, before.version + 1);
            let mut locator_after = before.locator_image;
            locator_after[..8].fill(0);
            locator_after[9] = RootState::ClosedPending as u8;
            encode_arena_ref(&mut locator_after[24..32], formation);
            write_u64(&mut locator_after, 56, before.version + 1);
            locator_after =
                self.wrappers
                    .prepare_reserved_image_after(wrapper, locator_after, 1)?;
            let formation_after = self.formations.prepare_reserved_image_after(
                formation,
                encode_topology_formation(
                    before,
                    RootState::ClosedPending,
                    FormationCause::MembershipConsumed,
                    SemanticOperation::MergeInitial,
                    event.id,
                    event.cancellation_fact,
                    event.generation_after,
                    index,
                    preview.occurred_at,
                    wrapper,
                ),
                1,
            )?;
            let mut funder_after = [[0; FUNDER_BYTES]; PLAN_MEMBERS_MAX];
            let mut member_after = [[0; MEMBER_BYTES]; PLAN_MEMBERS_MAX];
            for ordinal in 0..PLAN_MEMBERS_MAX {
                let member = before.members[ordinal];
                let next_funder = funders[index * PLAN_MEMBERS_MAX + ordinal];
                let mut funder = *self.funders.image(member.funder, &[1])?;
                funder[..8].fill(0);
                funder[10] = u8::try_from(before.version + 1).map_err(|_| capacity_error())?;
                encode_arena_ref(&mut funder[24..32], formation);
                let mut member_image = *self.members.image(member.member, &[1])?;
                encode_arena_ref(&mut member_image[24..32], next_funder);
                funder_after[ordinal] =
                    self.funders
                        .prepare_reserved_image_after(next_funder, funder, 1)?;
                member_after[ordinal] = member_image;
                push_local(
                    &mut local_entries,
                    &mut local_count,
                    LocalKind::Funder,
                    next_funder,
                )?;
            }
            let mutation_after = self.mutations.prepare_reserved_image_after(
                mutations[index],
                encode_topology_mutation(
                    SemanticOperation::MergeInitial,
                    event.id,
                    index,
                    before.group,
                    before.formation,
                    formation,
                    preview.occurred_at,
                    generation_after,
                ),
                1,
            )?;
            push_local(
                &mut local_entries,
                &mut local_count,
                LocalKind::Mutation,
                mutations[index],
            )?;
            source_journals[index] = Some(SourceJournal {
                before,
                locator_after_ref: wrapper,
                group_after,
                locator_after,
                formation_after,
                funder_after,
                member_after,
                mutation_after,
            });
        }

        let destination_group_ref = groups[0];
        let destination_formation_ref = formations[count];
        let destination_wrapper_ref = wrappers[count];
        let destination_member_refs: [ArenaRef; PLAN_MEMBERS_MAX] = members
            .as_slice()
            .try_into()
            .expect("four destination members");
        let destination_group = self.groups.prepare_reserved_image_after(
            destination_group_ref,
            encode_membership_group(
                STANDALONE_BRANCH,
                RootState::Pending,
                2,
                preview.destination.authority_key(),
                destination_formation_ref,
                destination_wrapper_ref,
                destination_member_refs,
                count,
            ),
            1,
        )?;
        let destination_formation = self.formations.prepare_reserved_image_after(
            destination_formation_ref,
            encode_merge_initial_destination_formation(
                &preview,
                &event,
                destination_group_ref,
                destination_wrapper_ref,
            ),
            1,
        )?;
        let destination_wrapper = self.wrappers.prepare_reserved_image_after(
            destination_wrapper_ref,
            encode_membership_wrapper(
                STANDALONE_BRANCH,
                RootState::Pending,
                destination_group_ref,
                destination_formation_ref,
                preview.destination.authority_key(),
                1,
            ),
            1,
        )?;
        let mut destination_funders = [[0; FUNDER_BYTES]; PLAN_MEMBERS_MAX];
        let mut destination_members = [[0; MEMBER_BYTES]; PLAN_MEMBERS_MAX];
        for ordinal in 0..PLAN_MEMBERS_MAX {
            let active = ordinal < count;
            let member = if active {
                preview.sources[ordinal].expect("active source").members[0]
            } else {
                preview.sources[0]
                    .expect("MergeInitial has an active source")
                    .members[0]
            };
            let funding = membership_funding(member)?;
            let funder = funders[count * PLAN_MEMBERS_MAX + ordinal];
            destination_funders[ordinal] = self.funders.prepare_reserved_image_after(
                funder,
                encode_membership_funder(
                    STANDALONE_BRANCH,
                    ordinal,
                    active,
                    destination_group_ref,
                    destination_formation_ref,
                    members[ordinal],
                    funding,
                    1,
                ),
                1,
            )?;
            destination_members[ordinal] = self.members.prepare_reserved_image_after(
                members[ordinal],
                encode_membership_member(
                    STANDALONE_BRANCH,
                    ordinal,
                    active,
                    destination_group_ref,
                    funder,
                    funding,
                ),
                1,
            )?;
            push_local(
                &mut local_entries,
                &mut local_count,
                LocalKind::Funder,
                funder,
            )?;
        }
        push_local(
            &mut local_entries,
            &mut local_count,
            LocalKind::Group,
            destination_group_ref,
        )?;

        let membership_image = self.memberships.prepare_reserved_image_after(
            memberships[0],
            encode_membership_event_image(
                SemanticOperation::MergeInitial,
                event.id,
                event.sources[0],
                preview.destination,
                event.affected[0]
                    .ok_or(SupportLedgerError::InvalidTransition)?
                    .key,
                generation_after,
                preview.occurred_at,
            ),
            1,
        )?;
        let membership_mutation = self.mutations.prepare_reserved_image_after(
            mutations[count],
            encode_membership_mutation(
                SemanticOperation::MergeInitial,
                event.id,
                destination_group_ref,
                destination_formation_ref,
                generation_after,
                preview.occurred_at,
            ),
            1,
        )?;
        push_local(
            &mut local_entries,
            &mut local_count,
            LocalKind::Membership,
            memberships[0],
        )?;
        push_local(
            &mut local_entries,
            &mut local_count,
            LocalKind::Mutation,
            mutations[count],
        )?;

        let authority_key = preview.destination.authority_key();
        let authority_before = self
            .authority
            .find(&authority_key)?
            .ok_or(SupportLedgerError::InvalidTransition)?;
        let authority_handle = self
            .authority
            .find_handle(&authority_key)?
            .ok_or_else(noncanonical_error)?;
        let prior_group = decode_arena_ref(&authority_before)?;
        let prior_group_image = self.groups.image(prior_group, &[1])?;
        if prior_group_image[40..57] != authority_key {
            return Err(noncanonical_error());
        }
        let authority_after = encode_arena_ref_value(destination_group_ref);
        self.authority.validate_update_batch(&[(
            authority_key,
            authority_handle,
            authority_after,
        )])?;

        let mut owner_records_after = owner_records;
        let mut owner_references = [[ArenaRef::default(); 4]; PLAN_MEMBERS_MAX];
        let mut owner_rows_after = [[0; OWNER_ROW_BYTES]; PLAN_MEMBERS_MAX];
        let mut owners_after = [[0; OWNER_BYTES]; PLAN_MEMBERS_MAX];
        let mut retired_links = [ArenaRef::default(); PLAN_MEMBERS_MAX];
        let mut retired_link_before = [[0; LINK_BYTES]; PLAN_MEMBERS_MAX];
        let mut retired_link_after = [[0; LINK_BYTES]; PLAN_MEMBERS_MAX];
        let mut replacement_links = [[0; LINK_BYTES]; PLAN_MEMBERS_MAX];
        for index in 0..preview.owner_count {
            let owner = preview.owners[index];
            let source = preview.sources[owner.source].expect("owner source");
            let before_record =
                owner_records[index].ok_or(SupportLedgerError::InvalidTransition)?;
            if before_record.request_owner != request_id_from_key_for_support(owner.request_key)?
                || owner.owner.slot != owner.slot
            {
                return Err(SupportLedgerError::InvalidTransition);
            }
            let references = [
                self.owner_headers.reference_at(owner.slot, &[1])?,
                self.owner_rows.reference_at(owner.slot, &[1])?,
                self.owner_indices.reference_at(owner.slot, &[1])?,
                self.owners.reference_at(owner.slot, &[1])?,
            ];
            if references[0] != owner.owner {
                return Err(noncanonical_error());
            }
            validate_c16_owner_set(
                [
                    self.owner_headers.image(references[0], &[1])?.as_slice(),
                    self.owner_rows.image(references[1], &[1])?.as_slice(),
                    self.owner_indices.image(references[2], &[1])?.as_slice(),
                    self.owners.image(references[3], &[1])?.as_slice(),
                ],
                references,
                owner.slot,
                &before_record,
                OWNER_STATE_LIVE,
            )?;
            let mut record_after = before_record;
            record_after.linked_claims =
                apply_i32_u32(record_after.linked_claims, owner.linked_delta)?;
            let mut row = *self.owner_rows.image(references[1], &[1])?;
            let mut owner_image = *self.owners.image(references[3], &[1])?;
            write_u32(
                &mut row,
                OWNER_ROW_LINKED_CLAIMS,
                record_after.linked_claims,
            );
            let current_after =
                apply_i32_u64(read_u64(&row, OWNER_ROW_CURRENT), owner.linked_delta)?;
            write_u64(&mut row, OWNER_ROW_CURRENT, current_after);
            for branch in 0..4 {
                let offset = OWNER_ROW_BRANCH_CURRENT + branch * 8;
                let branch_after =
                    apply_i32_u64(read_u64(&row, offset), owner.branch_delta[branch])?;
                write_u64(&mut row, offset, branch_after);
            }
            write_u32(
                &mut owner_image,
                OWNER_IMAGE_LINKED_CLAIMS,
                record_after.linked_claims,
            );
            let retired =
                decode_optional_arena_ref(&row[OWNER_ROW_ACTIVE_LINK..OWNER_ROW_ACTIVE_LINK + 8])?
                    .ok_or(SupportLedgerError::InvalidTransition)?;
            let before_link = *self.links.image(retired, &[1])?;
            if before_link[8] != 1
                || decode_arena_ref(&before_link[16..24])? != owner.owner
                || decode_arena_ref(&before_link[24..32])? != source.group
                || decode_arena_ref(&before_link[32..40])? != source.initial_formation
            {
                return Err(noncanonical_error());
            }
            let mut after_link = before_link;
            after_link[8] = 0;
            write_u64(&mut after_link, 80, generation_after);
            write_u64(&mut after_link, 88, preview.occurred_at);
            encode_arena_ref(
                &mut row[OWNER_ROW_ACTIVE_LINK..OWNER_ROW_ACTIVE_LINK + 8],
                links[index],
            );
            replacement_links[index] = self.links.prepare_reserved_image_after(
                links[index],
                encode_plan_link(
                    owner.owner,
                    destination_group_ref,
                    destination_formation_ref,
                    authority_key,
                    generation_after,
                ),
                1,
            )?;
            push_local(
                &mut local_entries,
                &mut local_count,
                LocalKind::Link,
                links[index],
            )?;
            owner_records_after[index] = Some(record_after);
            owner_references[index] = references;
            owner_rows_after[index] = row;
            owners_after[index] = owner_image;
            retired_links[index] = retired;
            retired_link_before[index] = before_link;
            retired_link_after[index] = after_link;
        }
        if owner_records[preview.owner_count..]
            .iter()
            .any(Option::is_some)
        {
            return Err(SupportLedgerError::InvalidInput);
        }

        local_entries[..local_count].sort_unstable_by_key(|entry| entry.0);
        if local_entries[..local_count]
            .windows(2)
            .any(|pair| pair[0].0 >= pair[1].0)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        self.local
            .validate_insert_batch(&local_entries[..local_count])?;
        let authority_plan = self.authority.prepare_update_assignment_plan(
            AUTHORITY_INDEX_ASSIGNMENT_ARENA,
            &[(authority_key, authority_handle, authority_after)],
        )?;
        let local_plan = self.local.prepare_insert_assignment_plan(
            LOCAL_INDEX_ASSIGNMENT_ARENA,
            &local_entries[..local_count],
        )?;
        let arena_headers_after = self.prepare_topology_arena_headers_after(
            &groups,
            &formations,
            &funders,
            &members,
            &wrappers,
            &links,
            &memberships,
            &mutations,
        )?;
        let next_merge_initial = read_u32(&self.header.0, 92)
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let mut header_after = self.header;
        write_u32(&mut header_after.0, 92, next_merge_initial);
        write_u64(&mut header_after.0, 48, generation_after);
        work.charge(HotPathWorkWitness::new(WORK_MERGE_INITIAL))?;
        Ok(PreparedMergeInitialTopology {
            expected_c17: self.generation(),
            expected_authority: self.authority.generation(),
            expected_local: self.local.generation(),
            expected_arena_headers,
            arena_headers_after,
            preview,
            event,
            groups,
            formations,
            funders,
            members,
            wrappers,
            links,
            memberships,
            mutations,
            source_journals,
            destination_group,
            destination_formation,
            destination_funders,
            destination_members,
            destination_wrapper,
            membership_image,
            membership_mutation,
            local_entries,
            local_count,
            authority_key,
            authority_before,
            authority_handle,
            authority_after,
            owner_records_before: owner_records,
            owner_records_after,
            owner_references,
            owner_rows_after,
            owners_after,
            retired_links,
            retired_link_before,
            retired_link_after,
            replacement_links,
            header_after,
            authority_plan,
            local_plan,
        })
    }

    pub(in crate::support) fn validate_merge_initial_topology(
        &self,
        change: &PreparedMergeInitialTopology,
        owner_records: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
    ) -> Result<(), SupportLedgerError> {
        self.validate_merge_initial_preview(&change.preview)?;
        validate_merge_initial_event(&change.preview, &change.event)?;
        let generation_after = self
            .generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let next_merge_initial = read_u32(&self.header.0, 92)
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let mut header_after = self.header;
        write_u32(&mut header_after.0, 92, next_merge_initial);
        write_u64(&mut header_after.0, 48, generation_after);
        let expected_arena_headers = self.topology_arena_headers();
        let count = change.preview.source_count;
        let arena_headers_after = self.prepare_topology_arena_headers_after(
            &change.groups,
            &change.formations,
            &change.funders,
            &change.members,
            &change.wrappers,
            &change.links,
            &change.memberships,
            &change.mutations,
        )?;
        if self.generation() != change.expected_c17
            || self.authority.generation() != change.expected_authority
            || self.local.generation() != change.expected_local
            || expected_arena_headers != change.expected_arena_headers
            || arena_headers_after != change.arena_headers_after
            || header_after != change.header_after
            || owner_records != change.owner_records_before
            || read_u32(&self.header.0, 92) >= MERGE_INITIAL_BUDGET as u32
            || self.groups.prepare_reserve::<1>(1)?.as_slice() != change.groups.as_slice()
            || self
                .formations
                .prepare_reserve::<FORMATION_MAX>(count + 1)?
                .as_slice()
                != change.formations.as_slice()
            || self
                .funders
                .prepare_reserve::<FUNDER_MAX>((count + 1) * PLAN_MEMBERS_MAX)?
                .as_slice()
                != change.funders.as_slice()
            || self
                .members
                .prepare_reserve::<MEMBER_MAX>(PLAN_MEMBERS_MAX)?
                .as_slice()
                != change.members.as_slice()
            || self
                .wrappers
                .prepare_reserve::<WRAPPER_MAX>(count + 1)?
                .as_slice()
                != change.wrappers.as_slice()
            || self
                .links
                .prepare_reserve::<LINK_MAX>(change.preview.owner_count)?
                .as_slice()
                != change.links.as_slice()
            || self.memberships.prepare_reserve::<1>(1)?.as_slice() != change.memberships.as_slice()
            || self
                .mutations
                .prepare_reserve::<MUTATION_MAX>(count + 1)?
                .as_slice()
                != change.mutations.as_slice()
            || self.authority.find(&change.authority_key)? != Some(change.authority_before)
            || change.local_entries[change.local_count..]
                .iter()
                .any(|entry| *entry != ([0; 17], [0; 8]))
            || !self
                .authority
                .validates_assignment_plan(&change.authority_plan)
            || !self.local.validates_assignment_plan(&change.local_plan)
        {
            return Err(SupportLedgerError::Generation);
        }
        self.authority.validate_update_batch(&[(
            change.authority_key,
            change.authority_handle,
            change.authority_after,
        )])?;
        self.local
            .validate_insert_batch(&change.local_entries[..change.local_count])?;
        for index in 0..change.preview.owner_count {
            let references = change.owner_references[index];
            validate_c16_owner_set(
                [
                    self.owner_headers.image(references[0], &[1])?.as_slice(),
                    self.owner_rows.image(references[1], &[1])?.as_slice(),
                    self.owner_indices.image(references[2], &[1])?.as_slice(),
                    self.owners.image(references[3], &[1])?.as_slice(),
                ],
                references,
                change.preview.owners[index].slot,
                &owner_records[index].ok_or(SupportLedgerError::Generation)?,
                OWNER_STATE_LIVE,
            )?;
            if self.links.image(change.retired_links[index], &[1])?
                != &change.retired_link_before[index]
            {
                return Err(SupportLedgerError::Generation);
            }
        }
        self.owner_rows.validate_advance_generation()?;
        self.owners.validate_advance_generation()?;
        let mut census = crate::work::ExactWorkCensus::new();
        let reconstructed = self.prepare_merge_initial_topology(
            change.preview,
            change.event,
            owner_records,
            &mut census,
        )?;
        if &reconstructed != change {
            return Err(SupportLedgerError::Generation);
        }
        Ok(())
    }

    pub(in crate::support) fn commit_merge_initial_topology(
        &mut self,
        change: PreparedMergeInitialTopology,
        owner_records: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
    ) {
        self.validate_merge_initial_topology(&change, owner_records)
            .expect("validated MergeInitial topology");
        self.commit_merge_initial_topology_prevalidated(change, true);
    }

    pub(in crate::support) fn commit_merge_initial_topology_prevalidated(
        &mut self,
        change: PreparedMergeInitialTopology,
        apply_index_plans: bool,
    ) {
        let count = change.preview.source_count;
        for index in 0..count {
            let journal = change.source_journals[index].expect("active source journal");
            self.formations
                .install_reserved_image_direct(change.formations[index], journal.formation_after);
            self.mutations
                .install_reserved_image_direct(change.mutations[index], journal.mutation_after);
            self.groups
                .replace_image_prevalidated(journal.before.group, journal.group_after);
            self.wrappers
                .install_reserved_image_direct(journal.locator_after_ref, journal.locator_after);
            for ordinal in 0..PLAN_MEMBERS_MAX {
                self.funders.install_reserved_image_direct(
                    change.formations_indexed_funder(index, ordinal),
                    journal.funder_after[ordinal],
                );
                self.members.replace_image_prevalidated(
                    journal.before.members[ordinal].member,
                    journal.member_after[ordinal],
                );
            }
        }
        let destination_group = change.groups[0];
        let destination_formation = change.formations[count];
        self.groups
            .install_reserved_image_direct(destination_group, change.destination_group);
        self.formations
            .install_reserved_image_direct(destination_formation, change.destination_formation);
        self.wrappers
            .install_reserved_image_direct(change.wrappers[count], change.destination_wrapper);
        for ordinal in 0..PLAN_MEMBERS_MAX {
            self.funders.install_reserved_image_direct(
                change.formations_indexed_funder(count, ordinal),
                change.destination_funders[ordinal],
            );
            self.members.install_reserved_image_direct(
                change.members[ordinal],
                change.destination_members[ordinal],
            );
        }
        self.memberships
            .install_reserved_image_direct(change.memberships[0], change.membership_image);
        self.mutations
            .install_reserved_image_direct(change.mutations[count], change.membership_mutation);
        for index in 0..change.preview.owner_count {
            let references = change.owner_references[index];
            self.owner_rows
                .replace_image_prevalidated(references[1], change.owner_rows_after[index]);
            self.owners
                .replace_image_prevalidated(references[3], change.owners_after[index]);
            self.links.replace_image_prevalidated(
                change.retired_links[index],
                change.retired_link_after[index],
            );
            self.links.install_reserved_image_direct(
                change.links[index],
                change.replacement_links[index],
            );
        }
        if apply_index_plans {
            self.authority
                .commit_assignment_plan_prevalidated(change.authority_plan);
            self.local
                .commit_assignment_plan_prevalidated(change.local_plan);
        }
        self.assign_topology_arena_headers(change.arena_headers_after);
        self.header = change.header_after;
    }

    fn prepare_topology_arena_headers_after(
        &self,
        groups: &ArenaSelection<1>,
        formations: &ArenaSelection<FORMATION_MAX>,
        funders: &ArenaSelection<FUNDER_MAX>,
        members: &ArenaSelection<MEMBER_MAX>,
        wrappers: &ArenaSelection<WRAPPER_MAX>,
        links: &ArenaSelection<LINK_MAX>,
        memberships: &ArenaSelection<1>,
        mutations: &ArenaSelection<MUTATION_MAX>,
    ) -> Result<[ByteArenaHeaderImage; 11], SupportLedgerError> {
        Ok([
            self.groups
                .prepare_reserve_header_after(groups, groups.len(), 0)?,
            self.formations
                .prepare_reserve_header_after(formations, formations.len(), 0)?,
            self.funders
                .prepare_reserve_header_after(funders, funders.len(), 0)?,
            self.members
                .prepare_reserve_header_after(members, members.len(), 0)?,
            self.wrappers
                .prepare_reserve_header_after(wrappers, wrappers.len(), 0)?,
            self.links
                .prepare_reserve_header_after(links, links.len(), 0)?,
            self.memberships
                .prepare_reserve_header_after(memberships, memberships.len(), 0)?,
            self.mutations
                .prepare_reserve_header_after(mutations, mutations.len(), 0)?,
            self.owner_rows.prepare_generation_header_after()?,
            self.owners.prepare_generation_header_after()?,
            self.external_heads.header_image(),
        ])
    }

    fn assign_topology_arena_headers(&mut self, headers: [ByteArenaHeaderImage; 11]) {
        self.groups.assign_header_direct(headers[0]);
        self.formations.assign_header_direct(headers[1]);
        self.funders.assign_header_direct(headers[2]);
        self.members.assign_header_direct(headers[3]);
        self.wrappers.assign_header_direct(headers[4]);
        self.links.assign_header_direct(headers[5]);
        self.memberships.assign_header_direct(headers[6]);
        self.mutations.assign_header_direct(headers[7]);
        self.owner_rows.assign_header_direct(headers[8]);
        self.owners.assign_header_direct(headers[9]);
        self.external_heads.assign_header_direct(headers[10]);
    }

    fn validate_merge_initial_preview(
        &self,
        preview: &MergeInitialPreview,
    ) -> Result<(), SupportLedgerError> {
        if self.generation() != preview.expected_c17
            || !(2..=SOURCE_MAX).contains(&preview.source_count)
            || preview.sources[..preview.source_count]
                .iter()
                .any(Option::is_none)
            || preview.sources[preview.source_count..]
                .iter()
                .any(Option::is_some)
            || preview.owner_count != preview.source_count
            || self.groups.prepare_reserve::<1>(1)?[0] != preview.destination.group()
        {
            return Err(SupportLedgerError::Generation);
        }
        for index in 0..preview.source_count {
            let before = preview.sources[index].expect("active source");
            let current = self.root_at_group(before.group, before.authority_key, before.branch)?;
            if current != before
                || preview.owners[index].request_key != before.members[0].request_key
            {
                return Err(SupportLedgerError::Generation);
            }
        }
        Ok(())
    }

    fn topology_arena_headers(&self) -> [ByteArenaHeaderImage; 11] {
        [
            self.groups.header_image(),
            self.formations.header_image(),
            self.funders.header_image(),
            self.members.header_image(),
            self.wrappers.header_image(),
            self.links.header_image(),
            self.memberships.header_image(),
            self.mutations.header_image(),
            self.owner_rows.header_image(),
            self.owners.header_image(),
            self.external_heads.header_image(),
        ]
    }
}

impl PreparedMergeInitialTopology {
    fn formations_indexed_funder(&self, formation: usize, ordinal: usize) -> ArenaRef {
        self.funders[formation * PLAN_MEMBERS_MAX + ordinal]
    }
}

fn validate_merge_initial_event(
    preview: &MergeInitialPreview,
    event: &MembershipEventRecord,
) -> Result<(), SupportLedgerError> {
    let count = preview.source_count;
    if event.kind != MembershipEventKind::MergeInitial
        || usize::from(event.source_count) != count
        || usize::from(event.member_count) != count
        || !event.consumed_by_support
        || event.occurred_at != preview.occurred_at
        || event.cancellation_fact != 0
        || event.affected[count..].iter().any(Option::is_some)
        || event.before[count..].iter().any(Option::is_some)
        || event.after[count..].iter().any(Option::is_some)
    {
        return Err(SupportLedgerError::InvalidTransition);
    }
    for index in 0..count {
        let source = preview.sources[index].expect("active source");
        let address = event.affected[index].ok_or(SupportLedgerError::InvalidTransition)?;
        let before = event.before[index].ok_or(SupportLedgerError::InvalidTransition)?;
        let after = event.after[index].ok_or(SupportLedgerError::InvalidTransition)?;
        if address.key != source.members[0].request_key
            || before.tag != MembershipTag::Bound
            || before.anchor.authority_key() != source.authority_key
            || before.anchor.group() != source.group
            || before.anchor.root() != source.group
            || after.tag != MembershipTag::Bound
            || after.anchor != preview.destination
            || after.epoch != before.epoch.checked_add(1).unwrap_or(0)
            || after.initial != before.initial
            || !after.pending.is_absent()
        {
            return Err(SupportLedgerError::InvalidTransition);
        }
    }
    Ok(())
}

fn materialize_pending_delta(member_count: usize) -> Result<AggregateDelta, SupportLedgerError> {
    let members = i32::try_from(member_count).map_err(|_| capacity_error())?;
    let attached = members.checked_sub(1).ok_or_else(noncanonical_error)?;
    let mut delta = AggregateDelta::ZERO;
    let pool = 1usize;
    for class in [1usize, 3usize] {
        delta.usage[class][pool] = 1;
        delta.reserved[class][pool] = -members;
        delta.attached[class][pool] = attached;
    }
    delta.usage[4][pool] = members;
    delta.reserved[4][pool] = -members;
    Ok(delta)
}

fn membership_funding(member: RootMemberSnapshot) -> Result<MembershipFunding, SupportLedgerError> {
    if !member.active {
        return Err(SupportLedgerError::InvalidInput);
    }
    Ok(MembershipFunding {
        request: request_id_from_key_for_support(member.request_key)?,
        request_key: member.request_key,
        record_slot: member.owner.slot,
        owner_header: member.owner,
        entitlement: member.entitlement,
        vector: member.vector,
        branch_limit: member.branch_limit,
    })
}

fn encode_topology_formation(
    before: RootSnapshot,
    after: RootState,
    cause: FormationCause,
    operation: SemanticOperation,
    event_id: u64,
    fact_id: u64,
    request_generation: u64,
    source_ordinal: usize,
    occurred_at: u64,
    locator: ArenaRef,
) -> [u8; FORMATION_BYTES] {
    let mut image = FormationImage::ZERO.0;
    encode_arena_ref(&mut image[8..16], before.initial_formation);
    encode_arena_ref(&mut image[16..24], before.formation);
    write_u64(&mut image, 24, event_id);
    write_u64(&mut image, 32, fact_id);
    image[40] = 0;
    image[41] = operation as u8;
    image[42] = source_ordinal as u8;
    write_u64(&mut image, 64, request_generation);
    image[104..121].copy_from_slice(&before.authority_key);
    image[220] = before.branch;
    image[221] = after as u8;
    image[222] = cause as u8;
    image[223] = 1;
    write_u64(&mut image, 224, before.version + 1);
    write_u64(&mut image, 232, occurred_at);
    encode_arena_ref(&mut image[240..248], before.group);
    encode_arena_ref(&mut image[248..256], locator);
    image
}

fn encode_merge_initial_destination_formation(
    preview: &MergeInitialPreview,
    event: &MembershipEventRecord,
    group: ArenaRef,
    wrapper: ArenaRef,
) -> [u8; FORMATION_BYTES] {
    let mut image = FormationImage::ZERO.0;
    write_u64(&mut image, 8, event.id);
    image[16] = event.source_count;
    for index in 0..preview.source_count {
        encode_arena_ref(
            &mut image[24 + index * 8..32 + index * 8],
            preview.sources[index].expect("source").group,
        );
    }
    image[104..121].copy_from_slice(&preview.destination.authority_key());
    image[220] = STANDALONE_BRANCH;
    image[221] = RootState::Pending as u8;
    image[222] = FormationCause::InitialReady as u8;
    image[223] = event.source_count;
    write_u64(&mut image, 224, 1);
    write_u64(&mut image, 232, preview.occurred_at);
    encode_arena_ref(&mut image[240..248], group);
    encode_arena_ref(&mut image[248..256], wrapper);
    image
}

fn encode_topology_mutation(
    operation: SemanticOperation,
    event_id: u64,
    ordinal: usize,
    group: ArenaRef,
    before: ArenaRef,
    after: ArenaRef,
    occurred_at: u64,
    generation: u64,
) -> [u8; MUTATION_BYTES] {
    let mut image = MutationImage::ZERO.0;
    image[8] = operation as u8;
    image[9] = ordinal as u8;
    write_u64(&mut image, 16, generation);
    write_u64(&mut image, 24, occurred_at);
    write_u64(&mut image, 32, event_id);
    encode_arena_ref(&mut image[40..48], group);
    encode_arena_ref(&mut image[48..56], before);
    encode_arena_ref(&mut image[56..64], after);
    image
}

fn push_local(
    entries: &mut [([u8; 17], [u8; 8]); LOCAL_MAX],
    count: &mut usize,
    kind: LocalKind,
    reference: ArenaRef,
) -> Result<(), SupportLedgerError> {
    if *count >= entries.len() {
        return Err(capacity_error());
    }
    entries[*count] = (
        local_key(kind, reference),
        encode_arena_ref_value(reference),
    );
    *count += 1;
    Ok(())
}

const SURGERY_DESTINATION_MAX: usize = 4;
const SURGERY_FORMATION_MAX: usize = SOURCE_MAX + SURGERY_DESTINATION_MAX;
const SURGERY_FUNDER_MAX: usize = SURGERY_FORMATION_MAX * PLAN_MEMBERS_MAX;
const SURGERY_MEMBER_MAX: usize = SURGERY_DESTINATION_MAX * PLAN_MEMBERS_MAX;
const SURGERY_WRAPPER_MAX: usize = SOURCE_MAX + 1;
const SURGERY_HEAD_MAX: usize = 3;
const SURGERY_LINK_MAX: usize = PLAN_MEMBERS_MAX;
const SURGERY_MUTATION_MAX: usize = SOURCE_MAX + 1;
const SURGERY_RAW_MAX: usize = SURGERY_HEAD_MAX * 2;
const SURGERY_AUTHORITY_MAX: usize = SURGERY_DESTINATION_MAX;
const SURGERY_LOCAL_MAX: usize = 48;
const SURGERY_LIFECYCLE_MAX: usize = 4;
const SURGERY_LIFECYCLE_RAW_MAX: usize = SURGERY_LIFECYCLE_MAX * 2;
const SURGERY_LIFECYCLE_PUBLICATION_MAX: usize = SURGERY_LIFECYCLE_MAX * PLAN_MEMBERS_MAX;
const LIFECYCLE_CLOSE_ACTION: u8 = 1;
const MEMBERSHIP_CLOSED_LIFECYCLE: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TopologyLifecycleSnapshot {
    reference: ArenaRef,
    record: LifecycleRecordInput,
    image: [u8; LIFECYCLE_BYTES],
    destination: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TopologyLifecycleJournal {
    snapshot: TopologyLifecycleSnapshot,
    after: [u8; LIFECYCLE_BYTES],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TopologyLifecycleRawUpdate {
    key: [u8; 32],
    handle: NodeHandle,
    before: [u8; 8],
    after: [u8; 8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TopologyDestination {
    anchor: SupportMembershipAnchor,
    locator_kind: u8,
    member_count: usize,
    members: [RootMemberSnapshot; PLAN_MEMBERS_MAX],
    obligation: [u8; 32],
    credit: [u8; 32],
}

impl TopologyDestination {
    const ZERO: Self = Self {
        anchor: SupportMembershipAnchor::ABSENT,
        locator_kind: 0,
        member_count: 0,
        members: [RootMemberSnapshot::ZERO; PLAN_MEMBERS_MAX],
        obligation: [0; 32],
        credit: [0; 32],
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MembershipTopologyPreview {
    expected_c17: u64,
    operation: SemanticOperation,
    sources: [Option<RootSnapshot>; SOURCE_MAX],
    source_count: usize,
    destinations: [TopologyDestination; SURGERY_DESTINATION_MAX],
    destination_count: usize,
    terminal_destination: bool,
    member_destinations: [u8; PLAN_MEMBERS_MAX],
    member_count: usize,
    aggregate: AggregateDelta,
    owners: [TopologyOwner; PLAN_MEMBERS_MAX],
    owner_count: usize,
    lifecycle_records: [Option<TopologyLifecycleSnapshot>; SURGERY_LIFECYCLE_MAX],
    lifecycle_record_count: usize,
    lifecycle_before: LifecycleAggregate,
    lifecycle_after: LifecycleAggregate,
    retractions: [LifecyclePublication; SURGERY_LIFECYCLE_PUBLICATION_MAX],
    retraction_count: usize,
    event_id: u64,
    request_generation: u64,
    occurred_at: u64,
}

impl MembershipTopologyPreview {
    pub(crate) fn destination_anchors(&self) -> [SupportMembershipAnchor; 4] {
        let mut anchors = [SupportMembershipAnchor::ABSENT; 4];
        for (index, destination) in self.destinations[..self.destination_count]
            .iter()
            .enumerate()
        {
            anchors[index] = destination.anchor;
        }
        anchors
    }

    pub(crate) const fn destination_count(&self) -> u8 {
        self.destination_count as u8
    }

    pub(crate) const fn terminal_destination(&self) -> bool {
        self.terminal_destination
    }

    pub(crate) fn source_member_keys(&self) -> [[u8; 40]; PLAN_MEMBERS_MAX] {
        let mut keys = [[0; 40]; PLAN_MEMBERS_MAX];
        for (index, owner) in self.owners[..self.owner_count].iter().enumerate() {
            keys[index] = owner.request_key;
        }
        keys
    }

    pub(crate) const fn source_member_count(&self) -> u8 {
        self.member_count as u8
    }

    pub(crate) const fn cancellation_survivor(&self) -> SupportMembershipAnchor {
        if self.terminal_destination || self.destination_count == 0 {
            SupportMembershipAnchor::ABSENT
        } else {
            self.destinations[0].anchor
        }
    }

    fn owner_has_resolver(&self, index: usize) -> bool {
        index < self.owner_count
            && !self.terminal_destination
            && self.destination_count > 0
            && usize::from(self.member_destinations[index]) < self.destination_count
    }

    fn replacement_link_count(&self) -> usize {
        (0..self.owner_count)
            .filter(|index| self.owner_has_resolver(*index))
            .count()
    }

    pub(crate) const fn aggregate_delta(&self) -> AggregateDelta {
        self.aggregate
    }

    pub(crate) const fn owner_count(&self) -> usize {
        self.owner_count
    }

    pub(crate) fn owner_slots(&self) -> [u32; PLAN_MEMBERS_MAX] {
        let mut slots = [0; PLAN_MEMBERS_MAX];
        for (index, owner) in self.owners[..self.owner_count].iter().enumerate() {
            slots[index] = owner.slot;
        }
        slots
    }

    pub(crate) fn owner_branch_delta(&self, index: usize) -> Option<[i32; 4]> {
        (index < self.owner_count).then_some(self.owners[index].vector_delta)
    }

    pub(in crate::support) const fn lifecycle_before(&self) -> LifecycleAggregate {
        self.lifecycle_before
    }

    pub(in crate::support) const fn lifecycle_after(&self) -> LifecycleAggregate {
        self.lifecycle_after
    }

    pub(in crate::support) fn retractions(&self) -> &[LifecyclePublication] {
        &self.retractions[..self.retraction_count]
    }
}

impl SupportC17 {
    pub(crate) fn inspect_membership_topology(
        &self,
        intent: &PreparedMembershipIntent,
    ) -> Result<MembershipTopologyPreview, SupportLedgerError> {
        let event = intent.event();
        let member_count = intent.member_count();
        let destination_count = intent.destination_count();
        if !(1..=PLAN_MEMBERS_MAX).contains(&member_count)
            || destination_count > SURGERY_DESTINATION_MAX
            || event.id != intent.event_id()
            || event.kind != intent.kind()
            || event.member_count as usize != member_count
            || event.occurred_at != intent.occurred_at()
            || event.occurred_at == 0
            || !event.consumed_by_support
            || event.cancellation_fact != 0
            || event.after.iter().any(Option::is_some)
            || event.affected[..member_count].iter().any(Option::is_none)
            || event.before[..member_count].iter().any(Option::is_none)
            || event.affected[member_count..].iter().any(Option::is_some)
            || event.before[member_count..].iter().any(Option::is_some)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        let operation = match (event.kind, event.source_count) {
            (MembershipEventKind::Join, 1) => SemanticOperation::NewlyEligibleJoin,
            (MembershipEventKind::Rebind, 0) => SemanticOperation::SourceFreeRebind,
            (MembershipEventKind::Rebind, 1) => SemanticOperation::NewlyEligibleRebind,
            (MembershipEventKind::Split, 0) => SemanticOperation::Split,
            (MembershipEventKind::Merge, 0) => SemanticOperation::Merge,
            (MembershipEventKind::Close, 0) => SemanticOperation::MembershipClose,
            _ => return Err(SupportLedgerError::InvalidTransition),
        };
        let expected_destination_count = match event.kind {
            MembershipEventKind::Join
            | MembershipEventKind::Rebind
            | MembershipEventKind::Merge => 1,
            MembershipEventKind::Split => 4,
            MembershipEventKind::Close => 0,
            _ => return Err(SupportLedgerError::InvalidTransition),
        };
        if destination_count != expected_destination_count {
            return Err(SupportLedgerError::InvalidTransition);
        }

        let mut source_anchors = [SupportMembershipAnchor::ABSENT; SOURCE_MAX];
        let mut source_count = 0usize;
        let mut member_destinations = [0; PLAN_MEMBERS_MAX];
        let mut used_destinations = [false; SURGERY_DESTINATION_MAX];
        for index in 0..member_count {
            let address = event.affected[index].ok_or(SupportLedgerError::InvalidInput)?;
            let before = event.before[index].ok_or(SupportLedgerError::InvalidInput)?;
            if !matches!(
                before.tag,
                MembershipTag::Bound | MembershipTag::EligibleUnbound
            ) || before.anchor.is_absent()
                || before.anchor.authority_key() == [0; 17]
                || before.anchor.branch() > 3
                || before.anchor.group() != before.anchor.root()
                || before.anchor.root_version() == 0
                || (index > 0
                    && event.affected[index - 1].is_none_or(|prior| prior.key >= address.key))
            {
                return Err(SupportLedgerError::InvalidInput);
            }
            if !source_anchors[..source_count].contains(&before.anchor) {
                if source_count == SOURCE_MAX {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                source_anchors[source_count] = before.anchor;
                source_count += 1;
            }
            match intent
                .destination(index)
                .ok_or(SupportLedgerError::InvalidInput)?
            {
                MembershipDestination::Destination(ordinal) => {
                    let ordinal = usize::from(ordinal);
                    if ordinal >= destination_count {
                        return Err(SupportLedgerError::InvalidInput);
                    }
                    member_destinations[index] = ordinal as u8;
                    used_destinations[ordinal] = true;
                }
                MembershipDestination::Closed => {
                    if event.kind != MembershipEventKind::Close {
                        return Err(SupportLedgerError::InvalidInput);
                    }
                    member_destinations[index] = u8::MAX;
                }
            }
        }
        if used_destinations[..destination_count]
            .iter()
            .any(|used| !used)
            || used_destinations[destination_count..]
                .iter()
                .any(|used| *used)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        let valid_source_count = match event.kind {
            MembershipEventKind::Join
            | MembershipEventKind::Rebind
            | MembershipEventKind::Split
            | MembershipEventKind::Close => source_count == 1,
            MembershipEventKind::Merge => (2..=SOURCE_MAX).contains(&source_count),
            _ => false,
        };
        if !valid_source_count {
            return Err(SupportLedgerError::InvalidTransition);
        }

        let mut sources = [None; SOURCE_MAX];
        for index in 0..source_count {
            let anchor = source_anchors[index];
            let root =
                self.root_at_group(anchor.group(), anchor.authority_key(), anchor.branch())?;
            if root.state != RootState::Pending
                || root.version < anchor.root_version()
                || event.occurred_at <= root.occurred_at
                || root.version == 4
            {
                return Err(SupportLedgerError::InvalidTransition);
            }
            sources[index] = Some(root);
        }
        sources[..source_count].sort_unstable_by_key(|source| {
            source.expect("active topology source").members[0].request_key
        });
        let branch = sources[0].expect("one topology source").branch;
        if sources[..source_count]
            .iter()
            .flatten()
            .any(|source| source.branch != branch)
        {
            return Err(SupportLedgerError::InvalidTransition);
        }

        let mut total_members = 0usize;
        let mut all_members = [RootMemberSnapshot::ZERO; PLAN_MEMBERS_MAX];
        for source in sources[..source_count].iter().copied().flatten() {
            total_members = total_members
                .checked_add(source.member_count)
                .ok_or_else(capacity_error)?;
            if total_members > PLAN_MEMBERS_MAX {
                return Err(SupportLedgerError::InvalidTransition);
            }
            for member in source.members[..source.member_count].iter().copied() {
                let event_index = event.affected[..member_count]
                    .iter()
                    .position(|address| {
                        address.is_some_and(|address| address.key == member.request_key)
                    })
                    .ok_or(SupportLedgerError::InvalidTransition)?;
                let before =
                    event.before[event_index].ok_or(SupportLedgerError::InvalidTransition)?;
                if before.anchor.group() != source.group
                    || before.anchor.authority_key() != source.authority_key
                    || before.anchor.branch() != source.branch
                {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                let occupied = all_members[..total_members]
                    .iter()
                    .position(|candidate| candidate.request_key == [0; 40])
                    .ok_or_else(noncanonical_error)?;
                all_members[occupied] = member;
            }
        }
        if total_members != member_count {
            return Err(SupportLedgerError::InvalidTransition);
        }
        all_members[..member_count].sort_unstable_by_key(|member| member.request_key);
        for index in 0..member_count {
            let address = event.affected[index].ok_or(SupportLedgerError::InvalidTransition)?;
            let member = all_members[index];
            if member.request_key != address.key
                || !member.active
                || all_members[..index].iter().any(|prior| {
                    prior.owner == member.owner
                        || prior.entitlement == member.entitlement
                        || prior.vector == member.vector
                })
            {
                return Err(SupportLedgerError::InvalidInput);
            }
        }

        if matches!(
            event.kind,
            MembershipEventKind::Rebind | MembershipEventKind::Split
        ) && sources[0].expect("source").locator_kind != 2
            || event.kind == MembershipEventKind::Merge
                && !sources[..source_count]
                    .iter()
                    .flatten()
                    .any(|source| source.locator_kind == 2)
        {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let initial_source = if matches!(
            event.kind,
            MembershipEventKind::Rebind | MembershipEventKind::Split | MembershipEventKind::Merge
        ) {
            sources[..source_count]
                .iter()
                .copied()
                .flatten()
                .filter(|source| source.locator_kind == 2)
                .min_by_key(|source| source.authority_key)
        } else {
            None
        };
        let initial_destination = match event.kind {
            MembershipEventKind::Rebind | MembershipEventKind::Merge => Some(0),
            MembershipEventKind::Split => Some(0),
            MembershipEventKind::Join | MembershipEventKind::Close => None,
            _ => return Err(SupportLedgerError::InvalidTransition),
        };
        if initial_destination.is_some() && initial_source.is_none() {
            return Err(SupportLedgerError::InvalidTransition);
        }

        let destination_groups = if destination_count == 0 {
            ArenaSelection::empty()
        } else {
            self.groups
                .prepare_reserve::<SURGERY_DESTINATION_MAX>(destination_count)?
        };
        let mut destinations = [TopologyDestination::ZERO; SURGERY_DESTINATION_MAX];
        for destination_index in 0..destination_count {
            let initial = initial_destination == Some(destination_index);
            let authority_key = if initial {
                initial_source
                    .expect("initial destination source")
                    .authority_key
            } else {
                topology_external_authority(event, destination_index)?
            };
            let group = destination_groups[destination_index];
            let anchor = SupportMembershipAnchor::try_new(
                authority_key,
                branch,
                group.slot,
                group.generation,
                group.slot,
                group.generation,
                1,
            )
            .map_err(|_| SupportLedgerError::InvalidInput)?;
            let (obligation, credit) = if initial {
                ([0; 32], [0; 32])
            } else {
                topology_external_raw(event, destination_index)?
            };
            let mut destination = TopologyDestination {
                anchor,
                locator_kind: if initial { 2 } else { 1 },
                member_count: 0,
                members: [RootMemberSnapshot::ZERO; PLAN_MEMBERS_MAX],
                obligation,
                credit,
            };
            for member_index in 0..member_count {
                if usize::from(member_destinations[member_index]) == destination_index {
                    destination.members[destination.member_count] = all_members[member_index];
                    destination.member_count += 1;
                }
            }
            if destination.member_count == 0 {
                return Err(SupportLedgerError::InvalidInput);
            }
            destinations[destination_index] = destination;
        }

        let mut aggregate = AggregateDelta::ZERO;
        for source in sources[..source_count].iter().copied().flatten() {
            aggregate.add(transition_aggregate(
                RootState::Pending,
                RootState::ClosedPending,
                source.member_count,
            )?)?;
        }
        for destination in &destinations[..destination_count] {
            aggregate.add(materialize_pending_delta(destination.member_count)?)?;
        }

        let mut owners = [TopologyOwner::ZERO; PLAN_MEMBERS_MAX];
        for index in 0..member_count {
            let member = all_members[index];
            let source = sources[..source_count]
                .iter()
                .position(|source| {
                    source.is_some_and(|source| {
                        source.members[..source.member_count]
                            .iter()
                            .any(|candidate| candidate.owner == member.owner)
                    })
                })
                .ok_or_else(noncanonical_error)?;
            let mut branch_delta = [0; 4];
            let linked_delta = if destination_count == 0 { -1 } else { 0 };
            if destination_count == 0 {
                branch_delta[usize::from(funding_branch(branch)?)] = -1;
            }
            owners[index] = TopologyOwner {
                slot: member.owner.slot,
                owner: member.owner,
                request_key: member.request_key,
                source,
                branch_delta,
                vector_delta: branch_delta,
                linked_delta,
            };
        }
        let (
            lifecycle_records,
            lifecycle_record_count,
            lifecycle_before,
            lifecycle_after,
            retractions,
            retraction_count,
        ) = self.inspect_topology_lifecycle(
            &sources,
            source_count,
            destination_count,
            false,
            &member_destinations,
            &mut owners,
        )?;
        add_topology_lifecycle_aggregate_delta(&mut aggregate, lifecycle_before, lifecycle_after)?;
        Ok(MembershipTopologyPreview {
            expected_c17: self.generation(),
            operation,
            sources,
            source_count,
            destinations,
            destination_count,
            terminal_destination: false,
            member_destinations,
            member_count,
            aggregate,
            owners,
            owner_count: member_count,
            lifecycle_records,
            lifecycle_record_count,
            lifecycle_before,
            lifecycle_after,
            retractions,
            retraction_count,
            event_id: event.id,
            request_generation: event.generation_after,
            occurred_at: event.occurred_at,
        })
    }

    pub(crate) fn inspect_cancellation_topology(
        &self,
        cancellation: &PreparedCancellation,
    ) -> Result<MembershipTopologyPreview, SupportLedgerError> {
        let event = *cancellation.event();
        let address = event.affected[0].ok_or(SupportLedgerError::InvalidTransition)?;
        let before = event.before[0].ok_or(SupportLedgerError::InvalidTransition)?;
        let after = event.after[0].ok_or(SupportLedgerError::InvalidTransition)?;
        let operation = match (before.tag, event.source_count) {
            (MembershipTag::Bound, 1) => SemanticOperation::CancellationRemoveBound,
            (MembershipTag::EligibleUnbound, 2) => {
                SemanticOperation::CancellationRemoveEligibleUnbound
            }
            _ => return Err(SupportLedgerError::InvalidTransition),
        };
        if event.kind != MembershipEventKind::CancellationRemove
            || event.member_count != 1
            || !event.consumed_by_support
            || event.id == 0
            || event.cancellation_fact == 0
            || event.occurred_at == 0
            || event.generation_after != event.generation_before.checked_add(1).unwrap_or(0)
            || address != cancellation.request()
            || before.anchor != cancellation.previous_anchor()
            || before.anchor.is_absent()
            || before.anchor.group() != before.anchor.root()
            || before.anchor.branch() > STANDALONE_BRANCH
            || after.tag != MembershipTag::Cancelled
            || after.epoch != before.epoch
            || !after.anchor.is_absent()
            || after.initial != before.initial
            || !after.pending.is_absent()
            || after.cancellation.is_absent()
            || after.cancellation_fact != event.cancellation_fact
            || after.cancellation_at != event.occurred_at
            || event.affected[1..].iter().any(Option::is_some)
            || event.before[1..].iter().any(Option::is_some)
            || event.after[1..].iter().any(Option::is_some)
        {
            return Err(SupportLedgerError::InvalidTransition);
        }

        let anchor = before.anchor;
        let root = self.root_at_group(anchor.group(), anchor.authority_key(), anchor.branch())?;
        let authority = self.authority.find(&root.authority_key)?;
        let authority_matches = if root.authority_key[0] == 0x32 {
            authority.is_none()
        } else {
            authority
                .map(|value| decode_arena_ref(&value))
                .transpose()?
                .is_some_and(|current| current == root.group)
        };
        if root.state != RootState::Pending
            || root.version < anchor.root_version()
            || root.version == 4
            || event.occurred_at <= root.occurred_at
            || !authority_matches
            || !(1..=PLAN_MEMBERS_MAX).contains(&root.member_count)
            || root.members[..root.member_count]
                .windows(2)
                .any(|pair| pair[0].request_key >= pair[1].request_key)
        {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let target_index = root.members[..root.member_count]
            .iter()
            .position(|member| member.request_key == address.key)
            .ok_or(SupportLedgerError::InvalidTransition)?;
        for (index, member) in root.members[..root.member_count].iter().enumerate() {
            if !member.active
                || root.members[..index].iter().any(|prior| {
                    prior.owner == member.owner
                        || prior.entitlement == member.entitlement
                        || prior.vector == member.vector
                })
            {
                return Err(SupportLedgerError::InvalidInput);
            }
        }

        let terminal_destination = root.member_count == 1;
        let destination_member_count = if terminal_destination {
            1
        } else {
            root.member_count - 1
        };
        let destination_group = self.groups.prepare_reserve::<1>(1)?[0];
        let destination_branch = if terminal_destination { 4 } else { root.branch };
        let destination_anchor = SupportMembershipAnchor::try_new(
            root.authority_key,
            destination_branch,
            destination_group.slot,
            destination_group.generation,
            destination_group.slot,
            destination_group.generation,
            1,
        )
        .map_err(|_| SupportLedgerError::InvalidTransition)?;
        let mut destination = TopologyDestination {
            anchor: destination_anchor,
            locator_kind: 2,
            member_count: destination_member_count,
            members: [RootMemberSnapshot::ZERO; PLAN_MEMBERS_MAX],
            obligation: [0; 32],
            credit: [0; 32],
        };
        if terminal_destination {
            destination.members[0] = root.members[target_index];
        } else {
            let mut destination_index = 0usize;
            for (index, member) in root.members[..root.member_count]
                .iter()
                .copied()
                .enumerate()
            {
                if index != target_index {
                    destination.members[destination_index] = member;
                    destination_index += 1;
                }
            }
            debug_assert_eq!(destination_index, destination_member_count);
        }

        let mut destinations = [TopologyDestination::ZERO; SURGERY_DESTINATION_MAX];
        destinations[0] = destination;
        let sources = [Some(root), None, None];
        let mut member_destinations = [0; PLAN_MEMBERS_MAX];
        let mut owners = [TopologyOwner::ZERO; PLAN_MEMBERS_MAX];
        let source_branch = usize::from(funding_branch(root.branch)?);
        for (index, member) in root.members[..root.member_count]
            .iter()
            .copied()
            .enumerate()
        {
            let removed = index == target_index;
            member_destinations[index] = if removed { u8::MAX } else { 0 };
            let root_delta = if removed && !terminal_destination {
                -1
            } else {
                0
            };
            let mut branch_delta = [0; 4];
            branch_delta[source_branch] = root_delta;
            owners[index] = TopologyOwner {
                slot: member.owner.slot,
                owner: member.owner,
                request_key: member.request_key,
                source: 0,
                branch_delta,
                vector_delta: branch_delta,
                linked_delta: root_delta,
            };
        }

        let mut aggregate = transition_aggregate(
            RootState::Pending,
            RootState::ClosedPending,
            root.member_count,
        )?;
        aggregate.add(materialize_pending_delta(destination_member_count)?)?;
        let (
            lifecycle_records,
            lifecycle_record_count,
            lifecycle_before,
            lifecycle_after,
            retractions,
            retraction_count,
        ) = self.inspect_topology_lifecycle(
            &sources,
            1,
            1,
            terminal_destination,
            &member_destinations,
            &mut owners,
        )?;
        add_topology_lifecycle_aggregate_delta(&mut aggregate, lifecycle_before, lifecycle_after)?;
        Ok(MembershipTopologyPreview {
            expected_c17: self.generation(),
            operation,
            sources,
            source_count: 1,
            destinations,
            destination_count: 1,
            terminal_destination,
            member_destinations,
            member_count: root.member_count,
            aggregate,
            owners,
            owner_count: root.member_count,
            lifecycle_records,
            lifecycle_record_count,
            lifecycle_before,
            lifecycle_after,
            retractions,
            retraction_count,
            event_id: event.id,
            request_generation: event.generation_after,
            occurred_at: event.occurred_at,
        })
    }

    fn inspect_topology_lifecycle(
        &self,
        sources: &[Option<RootSnapshot>; SOURCE_MAX],
        source_count: usize,
        destination_count: usize,
        terminal_destination: bool,
        member_destinations: &[u8; PLAN_MEMBERS_MAX],
        owners: &mut [TopologyOwner; PLAN_MEMBERS_MAX],
    ) -> Result<
        (
            [Option<TopologyLifecycleSnapshot>; SURGERY_LIFECYCLE_MAX],
            usize,
            LifecycleAggregate,
            LifecycleAggregate,
            [LifecyclePublication; SURGERY_LIFECYCLE_PUBLICATION_MAX],
            usize,
        ),
        SupportLedgerError,
    > {
        if self.pending_state()? != PendingState::Empty {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let owner_count = owners
            .iter()
            .position(|owner| *owner == TopologyOwner::ZERO)
            .unwrap_or(PLAN_MEMBERS_MAX);
        if owner_count == 0 {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let mut active_links = [ArenaRef::default(); PLAN_MEMBERS_MAX];
        for index in 0..owner_count {
            let owner = owners[index];
            let source = sources
                .get(owner.source)
                .copied()
                .flatten()
                .ok_or(SupportLedgerError::InvalidInput)?;
            let row = self
                .owner_rows
                .image(self.owner_rows.reference_at(owner.slot, &[1])?, &[1])?;
            let link =
                decode_optional_arena_ref(&row[OWNER_ROW_ACTIVE_LINK..OWNER_ROW_ACTIVE_LINK + 8])?
                    .ok_or(SupportLedgerError::InvalidTransition)?;
            let image = self.links.image(link, &[1])?;
            if image[8] != 1
                || decode_arena_ref(&image[16..24])? != owner.owner
                || decode_arena_ref(&image[24..32])? != source.group
                || decode_arena_ref(&image[32..40])? != source.initial_formation
            {
                return Err(noncanonical_error());
            }
            active_links[index] = link;
        }

        let mut records = [None; SURGERY_LIFECYCLE_MAX];
        let mut record_count = 0usize;
        let mut before = LifecycleAggregate::ZERO;
        let mut after = LifecycleAggregate::ZERO;
        let mut retractions = [LifecyclePublication::ZERO; SURGERY_LIFECYCLE_PUBLICATION_MAX];
        let mut retraction_count = 0usize;
        for slot in 0..self.lifecycle.capacity() {
            let Some(reference) = self.lifecycle.reference_if_occupied(slot as u32)? else {
                continue;
            };
            let image = *self.lifecycle.image(reference, &[1])?;
            if image[488] != 0 {
                validate_topology_closed_lifecycle_image(&image)?;
                continue;
            }
            let record = LifecycleRecordInput::decode(&image)?;
            self.validate_lifecycle_record_owner_set(record, reference)?;
            let lifecycle_owner_count = record
                .owners
                .iter()
                .position(|owner| *owner == LifecycleOwnerRow::ZERO)
                .unwrap_or(PLAN_MEMBERS_MAX);
            let linked = record.owners[..lifecycle_owner_count].iter().any(|owner| {
                decode_arena_ref(&owner.link.to_le_bytes())
                    .is_ok_and(|link| active_links[..owner_count].contains(&link))
            });
            if !linked {
                continue;
            }
            if record_count == records.len() || lifecycle_owner_count == 0 {
                return Err(capacity_error());
            }
            let mut destination = None;
            let mut close = destination_count == 0 || terminal_destination;
            for lifecycle_owner in record.owners[..lifecycle_owner_count].iter().copied() {
                let owner_ref = decode_arena_ref(&lifecycle_owner.owner.to_le_bytes())?;
                let owner_index = owners[..owner_count]
                    .iter()
                    .position(|owner| owner.owner == owner_ref)
                    .ok_or(SupportLedgerError::InvalidTransition)?;
                let link = decode_arena_ref(&lifecycle_owner.link.to_le_bytes())?;
                if active_links[owner_index] != link {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                if destination_count > 0 && !terminal_destination {
                    let current = member_destinations[owner_index];
                    if current == u8::MAX {
                        close = true;
                    } else if usize::from(current) >= destination_count {
                        return Err(SupportLedgerError::InvalidTransition);
                    } else if destination.is_some_and(|expected| expected != current) {
                        close = true;
                    } else if !close {
                        destination = Some(current);
                    }
                }
            }
            let destination = if close {
                u8::MAX
            } else {
                destination.ok_or(SupportLedgerError::InvalidTransition)?
            };
            for (kind, key) in [
                (RawOwnerKind::LifecycleObligation, record.obligation_raw),
                (RawOwnerKind::LifecycleCredit, record.credit_raw),
            ] {
                let raw = self.raw.find(&key)?.ok_or_else(noncanonical_error)?;
                let (actual_kind, state, owner) = decode_raw_owner(raw)?;
                if actual_kind != kind || state != RawOwnerState::Committed || owner != reference {
                    return Err(noncanonical_error());
                }
            }
            before.accrue(record)?;
            if destination != u8::MAX {
                after.accrue(record)?;
            } else {
                let branch = usize::from(funding_branch(record.final_owner[17])?);
                let axis = u8::try_from(record.aggregate[2]).map_err(|_| noncanonical_error())?;
                let horizon =
                    u8::try_from(record.aggregate[3]).map_err(|_| noncanonical_error())?;
                for lifecycle_owner in record.owners[..lifecycle_owner_count].iter().copied() {
                    let owner_ref = decode_arena_ref(&lifecycle_owner.owner.to_le_bytes())?;
                    let owner_index = owners[..owner_count]
                        .iter()
                        .position(|owner| owner.owner == owner_ref)
                        .ok_or_else(noncanonical_error)?;
                    owners[owner_index].linked_delta = owners[owner_index]
                        .linked_delta
                        .checked_sub(1)
                        .ok_or_else(capacity_error)?;
                    owners[owner_index].branch_delta[branch] = owners[owner_index].branch_delta
                        [branch]
                        .checked_sub(1)
                        .ok_or_else(capacity_error)?;
                    if retraction_count == retractions.len() {
                        return Err(capacity_error());
                    }
                    let member = decode_arena_ref(&lifecycle_owner.source.to_le_bytes())?;
                    let member_image = self.members.image(member, &[1])?;
                    retractions[retraction_count] = LifecyclePublication {
                        owner_slot: owner_ref.slot,
                        funder: decode_arena_ref(&member_image[24..32])?,
                        branch: record.final_owner[17],
                        axis,
                        horizon,
                        zero: 0,
                    };
                    retraction_count += 1;
                }
            }
            records[record_count] = Some(TopologyLifecycleSnapshot {
                reference,
                record,
                image,
                destination,
            });
            record_count += 1;
        }
        let _ = source_count;
        Ok((
            records,
            record_count,
            before,
            after,
            retractions,
            retraction_count,
        ))
    }
}

fn arena_ref_word(reference: ArenaRef) -> u64 {
    u64::from_le_bytes(encode_arena_ref_value(reference))
}

fn adjust_topology_funder_current(
    image: &mut [u8; FUNDER_BYTES],
    delta: i32,
) -> Result<(), SupportLedgerError> {
    let after = apply_i32_u64(read_u64(image, 112), delta)?;
    if after > read_u64(image, 120) {
        return Err(capacity_error());
    }
    write_u64(image, 112, after);
    Ok(())
}

fn add_topology_lifecycle_aggregate_delta(
    aggregate: &mut AggregateDelta,
    before: LifecycleAggregate,
    after: LifecycleAggregate,
) -> Result<(), SupportLedgerError> {
    let mut delta = AggregateDelta::ZERO;
    for class in 0..5 {
        for pool in 0..3 {
            let usage = i64::from(after.usage[class][pool])
                .checked_sub(i64::from(before.usage[class][pool]))
                .ok_or_else(capacity_error)?;
            let reserved = i64::from(before.reserved[class][pool])
                .checked_sub(i64::from(after.reserved[class][pool]))
                .ok_or_else(capacity_error)?;
            delta.usage[class][pool] = i32::try_from(usage).map_err(|_| capacity_error())?;
            delta.reserved[class][pool] = i32::try_from(reserved).map_err(|_| capacity_error())?;
            if class < 4 {
                let attached = i64::from(after.attached[class][pool])
                    .checked_sub(i64::from(before.attached[class][pool]))
                    .ok_or_else(capacity_error)?;
                delta.attached[class][pool] =
                    i32::try_from(attached).map_err(|_| capacity_error())?;
            }
        }
    }
    aggregate.add(delta)
}

fn validate_topology_closed_lifecycle_image(
    image: &[u8; LIFECYCLE_BYTES],
) -> Result<(), SupportLedgerError> {
    (image[488] == LIFECYCLE_CLOSE_ACTION
        && matches!(image[489], 1 | MEMBERSHIP_CLOSED_LIFECYCLE)
        && image[490..496].iter().all(|byte| *byte == 0)
        && read_u64(image, 496) != 0
        && read_u64(image, 504) != 0
        && image[1_024..].iter().all(|byte| *byte == 0))
    .then_some(())
    .ok_or_else(noncanonical_error)
}

fn topology_external_authority(
    event: &MembershipEventRecord,
    destination: usize,
) -> Result<[u8; 17], SupportLedgerError> {
    let ordinal = u8::try_from(destination).map_err(|_| SupportLedgerError::InvalidInput)?;
    let mut key = [0; 17];
    key[0] = 0x32;
    key[1..9].copy_from_slice(&event.id.to_be_bytes());
    key[9] = ordinal.checked_add(1).ok_or_else(capacity_error)?;
    key[10] = event.kind as u8;
    key[11..17].copy_from_slice(&event.generation_after.to_be_bytes()[2..]);
    Ok(key)
}

fn topology_external_raw(
    event: &MembershipEventRecord,
    destination: usize,
) -> Result<([u8; 32], [u8; 32]), SupportLedgerError> {
    let ordinal = u8::try_from(destination).map_err(|_| SupportLedgerError::InvalidInput)?;
    let mut obligation = [0; 32];
    obligation[0] = 0x71;
    obligation[1..9].copy_from_slice(&event.id.to_be_bytes());
    obligation[9] = ordinal.checked_add(1).ok_or_else(capacity_error)?;
    obligation[10] = event.kind as u8;
    obligation[11..19].copy_from_slice(&event.generation_after.to_be_bytes());
    let mut credit = obligation;
    credit[0] = 0x72;
    Ok((obligation, credit))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TopologyDestinationJournal {
    group: ArenaRef,
    formation: ArenaRef,
    locator: ArenaRef,
    locator_kind: u8,
    group_image: [u8; GROUP_BYTES],
    formation_image: [u8; FORMATION_BYTES],
    locator_image: [u8; EXTERNAL_HEAD_BYTES],
    funder_images: [[u8; FUNDER_BYTES]; PLAN_MEMBERS_MAX],
    member_images: [[u8; MEMBER_BYTES]; PLAN_MEMBERS_MAX],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TopologyAuthorityUpdate {
    key: [u8; 17],
    before: [u8; 8],
    handle: NodeHandle,
    after: [u8; 8],
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PreparedMembershipTopology {
    expected_c17: u64,
    expected_raw: u64,
    expected_authority: u64,
    expected_local: u64,
    expected_lifecycle: u64,
    expected_arena_headers: [ByteArenaHeaderImage; 12],
    arena_headers_after: [ByteArenaHeaderImage; 12],
    preview: MembershipTopologyPreview,
    event: MembershipEventRecord,
    groups: ArenaSelection<SURGERY_DESTINATION_MAX>,
    external_heads: ArenaSelection<SURGERY_HEAD_MAX>,
    external_head_count: usize,
    formations: ArenaSelection<SURGERY_FORMATION_MAX>,
    funders: ArenaSelection<SURGERY_FUNDER_MAX>,
    members: ArenaSelection<SURGERY_MEMBER_MAX>,
    wrappers: ArenaSelection<SURGERY_WRAPPER_MAX>,
    wrapper_count: usize,
    links: ArenaSelection<SURGERY_LINK_MAX>,
    link_count: usize,
    replacement_link_indices: [u8; PLAN_MEMBERS_MAX],
    memberships: ArenaSelection<1>,
    mutations: ArenaSelection<SURGERY_MUTATION_MAX>,
    source_journals: [Option<SourceJournal>; SOURCE_MAX],
    destination_journals: [Option<TopologyDestinationJournal>; SURGERY_DESTINATION_MAX],
    membership_image: [u8; MEMBERSHIP_BYTES],
    membership_mutation: [u8; MUTATION_BYTES],
    raw_entries: [([u8; 32], [u8; 8]); SURGERY_RAW_MAX],
    raw_count: usize,
    authority_inserts: [([u8; 17], [u8; 8]); SURGERY_AUTHORITY_MAX],
    authority_insert_count: usize,
    authority_updates: [Option<TopologyAuthorityUpdate>; 1],
    authority_update_count: usize,
    local_entries: [([u8; 17], [u8; 8]); SURGERY_LOCAL_MAX],
    local_count: usize,
    owner_records_before: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
    owner_records_after: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
    owner_references: [[ArenaRef; 4]; PLAN_MEMBERS_MAX],
    owner_rows_after: [[u8; OWNER_ROW_BYTES]; PLAN_MEMBERS_MAX],
    owners_after: [[u8; OWNER_BYTES]; PLAN_MEMBERS_MAX],
    retired_links: [ArenaRef; PLAN_MEMBERS_MAX],
    retired_link_before: [[u8; LINK_BYTES]; PLAN_MEMBERS_MAX],
    retired_link_after: [[u8; LINK_BYTES]; PLAN_MEMBERS_MAX],
    replacement_links: [[u8; LINK_BYTES]; PLAN_MEMBERS_MAX],
    header_after: C17HeaderImage,
    lifecycle_journals: [Option<TopologyLifecycleJournal>; SURGERY_LIFECYCLE_MAX],
    lifecycle_raw_updates: [Option<TopologyLifecycleRawUpdate>; SURGERY_LIFECYCLE_RAW_MAX],
    lifecycle_raw_update_count: usize,
    raw_plan: Option<PatriciaAssignmentPlan<RAW_ASSIGNMENT_MAX>>,
    authority_plan: Option<PatriciaAssignmentPlan<AUTHORITY_ASSIGNMENT_MAX>>,
    local_plan: PatriciaAssignmentPlan<LOCAL_ASSIGNMENT_MAX>,
}

impl PreparedMembershipTopology {
    pub(crate) const fn aggregate_delta(&self) -> AggregateDelta {
        self.preview.aggregate
    }

    pub(crate) const fn owner_count(&self) -> usize {
        self.preview.owner_count
    }

    pub(crate) fn owner_slots(&self) -> [u32; PLAN_MEMBERS_MAX] {
        self.preview.owner_slots()
    }

    pub(crate) fn owner_branch_delta(&self, index: usize) -> Option<[i32; 4]> {
        self.preview.owner_branch_delta(index)
    }

    pub(in crate::support) const fn owner_records_after(
        &self,
    ) -> [Option<BundleRecord>; PLAN_MEMBERS_MAX] {
        self.owner_records_after
    }

    pub(in crate::support) const fn lifecycle_before(&self) -> LifecycleAggregate {
        self.preview.lifecycle_before
    }

    pub(in crate::support) const fn lifecycle_after(&self) -> LifecycleAggregate {
        self.preview.lifecycle_after
    }

    pub(in crate::support) fn retractions(&self) -> &[LifecyclePublication] {
        self.preview.retractions()
    }

    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        if let Some(plan) = &self.raw_plan {
            plan.visit_assignments(visitor);
        }
        if let Some(plan) = &self.authority_plan {
            plan.visit_assignments(visitor);
        }
        self.local_plan.visit_assignments(visitor);
    }
}

impl SupportC17 {
    pub(in crate::support) fn prepare_membership_topology(
        &self,
        preview: MembershipTopologyPreview,
        event: MembershipEventRecord,
        owner_records: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
        work: &mut impl WorkRecorder,
    ) -> Result<PreparedMembershipTopology, SupportLedgerError> {
        self.validate_membership_topology_preview(&preview)?;
        validate_membership_topology_event(&preview, &event)?;
        let generation_after = self
            .generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        if read_u32(&self.header.0, 96) >= POST_CREATE_BUDGET as u32 {
            return Err(capacity_error());
        }
        let source_count = preview.source_count;
        let destination_count = preview.destination_count;
        let formation_count = source_count
            .checked_add(destination_count)
            .ok_or_else(capacity_error)?;
        let groups = if destination_count == 0 {
            ArenaSelection::empty()
        } else {
            self.groups
                .prepare_reserve::<SURGERY_DESTINATION_MAX>(destination_count)?
        };
        for index in 0..destination_count {
            if groups[index] != preview.destinations[index].anchor.group() {
                return Err(SupportLedgerError::Generation);
            }
        }
        let external_head_count = preview.destinations[..destination_count]
            .iter()
            .filter(|destination| destination.locator_kind == 1)
            .count();
        let external_heads = if external_head_count == 0 {
            ArenaSelection::empty()
        } else {
            self.external_heads
                .prepare_reserve::<SURGERY_HEAD_MAX>(external_head_count)?
        };
        let formations = self
            .formations
            .prepare_reserve::<SURGERY_FORMATION_MAX>(formation_count)?;
        let funders = self
            .funders
            .prepare_reserve::<SURGERY_FUNDER_MAX>(formation_count * PLAN_MEMBERS_MAX)?;
        let members = if destination_count == 0 {
            ArenaSelection::empty()
        } else {
            self.members
                .prepare_reserve::<SURGERY_MEMBER_MAX>(destination_count * PLAN_MEMBERS_MAX)?
        };
        let source_wrapper_count = preview.sources[..source_count]
            .iter()
            .flatten()
            .filter(|source| source.locator_kind == 2)
            .count();
        let destination_wrapper_count = preview.destinations[..destination_count]
            .iter()
            .filter(|destination| destination.locator_kind == 2)
            .count();
        let wrapper_count = source_wrapper_count
            .checked_add(destination_wrapper_count)
            .ok_or_else(capacity_error)?;
        let wrappers = if wrapper_count == 0 {
            ArenaSelection::empty()
        } else {
            self.wrappers
                .prepare_reserve::<SURGERY_WRAPPER_MAX>(wrapper_count)?
        };
        let link_count = preview.replacement_link_count();
        let mut replacement_link_indices = [u8::MAX; PLAN_MEMBERS_MAX];
        let mut replacement_index = 0usize;
        for (owner_index, slot) in replacement_link_indices
            .iter_mut()
            .enumerate()
            .take(preview.owner_count)
        {
            if preview.owner_has_resolver(owner_index) {
                *slot = u8::try_from(replacement_index).map_err(|_| capacity_error())?;
                replacement_index += 1;
            }
        }
        debug_assert_eq!(replacement_index, link_count);
        let links = if link_count == 0 {
            ArenaSelection::empty()
        } else {
            self.links.prepare_reserve::<SURGERY_LINK_MAX>(link_count)?
        };
        let memberships = self.memberships.prepare_reserve::<1>(1)?;
        let mutations = self
            .mutations
            .prepare_reserve::<SURGERY_MUTATION_MAX>(source_count + 1)?;

        let mut raw_entries = [([0; 32], [0; 8]); SURGERY_RAW_MAX];
        let mut raw_count = 0usize;
        let mut authority_inserts = [([0; 17], [0; 8]); SURGERY_AUTHORITY_MAX];
        let mut authority_insert_count = 0usize;
        let mut authority_updates = [None; 1];
        let mut authority_update_count = 0usize;
        let mut external_index = 0usize;
        for destination_index in 0..destination_count {
            let destination = preview.destinations[destination_index];
            let group = groups[destination_index];
            if destination.locator_kind == 1 {
                let head = external_heads[external_index];
                external_index += 1;
                if self
                    .authority
                    .find(&destination.anchor.authority_key())?
                    .is_some()
                {
                    return Err(SupportLedgerError::Storage(FixedStorageError::Duplicate));
                }
                for (kind, key) in [
                    (RawOwnerKind::PlanRoot, destination.obligation),
                    (RawOwnerKind::Formation, destination.credit),
                ] {
                    raw_entries[raw_count] = (
                        key,
                        encode_raw_owner_at(
                            kind,
                            RawOwnerState::Committed,
                            destination.anchor.branch(),
                            head,
                        )?,
                    );
                    raw_count += 1;
                }
            } else {
                let key = destination.anchor.authority_key();
                if let Some(before) = self.authority.find(&key)? {
                    if authority_update_count == authority_updates.len() {
                        return Err(capacity_error());
                    }
                    let handle = self
                        .authority
                        .find_handle(&key)?
                        .ok_or_else(noncanonical_error)?;
                    let prior = decode_arena_ref(&before)?;
                    let prior_image = self.groups.image(prior, &[1])?;
                    if prior_image[40..57] != key {
                        return Err(noncanonical_error());
                    }
                    authority_updates[authority_update_count] = Some(TopologyAuthorityUpdate {
                        key,
                        before,
                        handle,
                        after: encode_arena_ref_value(group),
                    });
                    authority_update_count += 1;
                } else if key[0] == 0x32 {
                    if authority_insert_count == authority_inserts.len() {
                        return Err(capacity_error());
                    }
                    authority_inserts[authority_insert_count] =
                        (key, encode_arena_ref_value(group));
                    authority_insert_count += 1;
                } else {
                    return Err(SupportLedgerError::InvalidTransition);
                }
            }
        }
        authority_inserts[..authority_insert_count].sort_unstable_by_key(|entry| entry.0);
        raw_entries[..raw_count].sort_unstable_by_key(|entry| entry.0);
        if authority_inserts[..authority_insert_count]
            .windows(2)
            .any(|pair| pair[0].0 >= pair[1].0)
            || raw_entries[..raw_count]
                .windows(2)
                .any(|pair| pair[0].0 >= pair[1].0)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        if authority_insert_count > 0 {
            self.authority
                .validate_insert_batch(&authority_inserts[..authority_insert_count])?;
        }
        if let Some(update) = authority_updates[0] {
            self.authority
                .validate_update_batch(&[(update.key, update.handle, update.after)])?;
        }
        if raw_count > 0 {
            self.raw.validate_insert_batch(&raw_entries[..raw_count])?;
        }

        let mut source_journals = [None; SOURCE_MAX];
        let mut destination_journals = [None; SURGERY_DESTINATION_MAX];
        let mut local_entries = [([0; 17], [0; 8]); SURGERY_LOCAL_MAX];
        let mut local_count = 0usize;
        let mut source_wrapper_index = 0usize;
        for source_index in 0..source_count {
            let before = preview.sources[source_index].expect("active topology source");
            let formation = formations[source_index];
            let locator_after_ref = if before.locator_kind == 1 {
                before.locator
            } else {
                let wrapper = wrappers[source_wrapper_index];
                source_wrapper_index += 1;
                wrapper
            };
            let mut group_after = before.group_image;
            group_after[9] = RootState::ClosedPending as u8;
            encode_arena_ref(&mut group_after[16..24], formation);
            encode_arena_ref(&mut group_after[24..32], locator_after_ref);
            write_u64(&mut group_after, 32, before.version + 1);
            let mut locator_after = before.locator_image;
            if before.locator_kind == 2 {
                locator_after[..8].fill(0);
            }
            locator_after[9] = RootState::ClosedPending as u8;
            encode_arena_ref(&mut locator_after[24..32], formation);
            write_u64(
                &mut locator_after,
                if before.locator_kind == 1 { 120 } else { 56 },
                before.version + 1,
            );
            if before.locator_kind == 2 {
                locator_after = self.wrappers.prepare_reserved_image_after(
                    locator_after_ref,
                    locator_after,
                    1,
                )?;
            }
            let cancellation = matches!(
                preview.operation,
                SemanticOperation::CancellationRemoveBound
                    | SemanticOperation::CancellationRemoveEligibleUnbound
            );
            let formation_after = self.formations.prepare_reserved_image_after(
                formation,
                encode_topology_formation(
                    before,
                    RootState::ClosedPending,
                    if cancellation {
                        FormationCause::CancellationMembership
                    } else {
                        FormationCause::MembershipConsumed
                    },
                    preview.operation,
                    event.id,
                    if cancellation {
                        event.cancellation_fact
                    } else {
                        0
                    },
                    event.generation_after,
                    source_index,
                    preview.occurred_at,
                    locator_after_ref,
                ),
                1,
            )?;
            let mut funder_after = [[0; FUNDER_BYTES]; PLAN_MEMBERS_MAX];
            let mut member_after = [[0; MEMBER_BYTES]; PLAN_MEMBERS_MAX];
            for ordinal in 0..PLAN_MEMBERS_MAX {
                let member = before.members[ordinal];
                let next_funder = funders[source_index * PLAN_MEMBERS_MAX + ordinal];
                let mut funder = *self.funders.image(member.funder, &[1])?;
                funder[..8].fill(0);
                funder[10] = u8::try_from(before.version + 1).map_err(|_| capacity_error())?;
                encode_arena_ref(&mut funder[24..32], formation);
                let mut member_image = *self.members.image(member.member, &[1])?;
                encode_arena_ref(&mut member_image[24..32], next_funder);
                funder_after[ordinal] =
                    self.funders
                        .prepare_reserved_image_after(next_funder, funder, 1)?;
                member_after[ordinal] = member_image;
                push_surgery_local(
                    &mut local_entries,
                    &mut local_count,
                    LocalKind::Funder,
                    next_funder,
                )?;
            }
            let mutation_after = self.mutations.prepare_reserved_image_after(
                mutations[source_index],
                encode_topology_mutation(
                    preview.operation,
                    event.id,
                    source_index,
                    before.group,
                    before.formation,
                    formation,
                    preview.occurred_at,
                    generation_after,
                ),
                1,
            )?;
            push_surgery_local(
                &mut local_entries,
                &mut local_count,
                LocalKind::Mutation,
                mutations[source_index],
            )?;
            source_journals[source_index] = Some(SourceJournal {
                before,
                locator_after_ref,
                group_after,
                locator_after,
                formation_after,
                funder_after,
                member_after,
                mutation_after,
            });
        }

        let mut destination_wrapper_index = source_wrapper_count;
        external_index = 0;
        for destination_index in 0..destination_count {
            let destination = preview.destinations[destination_index];
            let formation_index = source_count + destination_index;
            let group = groups[destination_index];
            let formation = formations[formation_index];
            let locator = if destination.locator_kind == 1 {
                let head = external_heads[external_index];
                external_index += 1;
                head
            } else {
                let wrapper = wrappers[destination_wrapper_index];
                destination_wrapper_index += 1;
                wrapper
            };
            let member_refs: [ArenaRef; PLAN_MEMBERS_MAX] = members.as_slice()
                [destination_index * PLAN_MEMBERS_MAX..(destination_index + 1) * PLAN_MEMBERS_MAX]
                .try_into()
                .expect("fixed destination Member range");
            let group_image = self.groups.prepare_reserved_image_after(
                group,
                encode_membership_group(
                    destination.anchor.branch(),
                    RootState::Pending,
                    destination.locator_kind,
                    destination.anchor.authority_key(),
                    formation,
                    locator,
                    member_refs,
                    destination.member_count,
                ),
                1,
            )?;
            let formation_image = self.formations.prepare_reserved_image_after(
                formation,
                encode_membership_destination_formation(
                    &preview,
                    &event,
                    destination_index,
                    group,
                    formation,
                    locator,
                ),
                1,
            )?;
            let locator_image = if destination.locator_kind == 1 {
                self.external_heads.prepare_reserved_image_after(
                    locator,
                    encode_membership_external_head(destination, group, formation),
                    1,
                )?
            } else {
                self.wrappers.prepare_reserved_image_after(
                    locator,
                    encode_membership_wrapper(
                        destination.anchor.branch(),
                        RootState::Pending,
                        group,
                        formation,
                        destination.anchor.authority_key(),
                        1,
                    ),
                    1,
                )?
            };
            let mut funder_images = [[0; FUNDER_BYTES]; PLAN_MEMBERS_MAX];
            let mut member_images = [[0; MEMBER_BYTES]; PLAN_MEMBERS_MAX];
            for ordinal in 0..PLAN_MEMBERS_MAX {
                let active = ordinal < destination.member_count;
                let member = if active {
                    destination.members[ordinal]
                } else {
                    destination.members[0]
                };
                let funding = membership_funding(member)?;
                let funder = funders[formation_index * PLAN_MEMBERS_MAX + ordinal];
                let member_ref = member_refs[ordinal];
                funder_images[ordinal] = self.funders.prepare_reserved_image_after(
                    funder,
                    encode_membership_funder(
                        destination.anchor.branch(),
                        ordinal,
                        active,
                        group,
                        formation,
                        member_ref,
                        funding,
                        1,
                    ),
            }
        }
    }
}
