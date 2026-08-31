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
