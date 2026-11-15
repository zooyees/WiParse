//! Synchronisation primitive for host-owned AI sources. This does not make an
//! external file, device, or a polling snapshot authoritative by itself.
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct SourceLease { state: Arc<Mutex<LeaseState>> }

#[derive(Default)]
struct LeaseState { generation:u64, active_updates:usize }

/// Marks a non-locking update interval. Dropping it finishes the interval;
/// leaked guards intentionally keep new snapshots and dispatches blocked.
#[must_use]
pub struct SourceUpdate { lease:SourceLease, active:bool }

#[derive(Clone)]
pub struct SourcePermit { lease: SourceLease, generation: u64 }

impl SourceLease {
    /// Publish state guarded by two leases using the same global lock order as
    /// multi-source dispatch. Mutations remain lossless on poison/exhaustion.
    pub fn mutate_pair_always<T>(a:&Self,b:&Self,mutation:impl FnOnce()->T)->T {
        if Arc::ptr_eq(&a.state,&b.state) {return a.mutate_always(mutation);}
        let (first,second)=if (Arc::as_ptr(&a.state) as usize)<(Arc::as_ptr(&b.state) as usize) {(a,b)} else {(b,a)};
        let mut one=first.state.lock().unwrap_or_else(|poisoned|{let mut state=poisoned.into_inner();state.generation=u64::MAX;state});
        let mut two=second.state.lock().unwrap_or_else(|poisoned|{let mut state=poisoned.into_inner();state.generation=u64::MAX;state});
        one.generation=one.generation.checked_add(1).unwrap_or(u64::MAX);
        two.generation=two.generation.checked_add(1).unwrap_or(u64::MAX);
        mutation()
    }
    /// Mutate three related sources under the same address ordering used by
    /// dispatch. Duplicate leases are acquired and advanced only once.
    pub fn mutate_three_always<T>(a:&Self,b:&Self,c:&Self,mutation:impl FnOnce()->T)->T {
        let mut ordered=vec![a,b,c];
        ordered.sort_by_key(|lease|Arc::as_ptr(&lease.state) as usize);
        ordered.dedup_by(|left,right|Arc::ptr_eq(&left.state,&right.state));
        fn acquire<T,F:FnOnce()->T>(ordered:&[&SourceLease],mutation:&mut Option<F>)->T {
            let Some((first,rest))=ordered.split_first() else {return mutation.take().unwrap()();};
            let mut state=first.state.lock().unwrap_or_else(|poisoned|{let mut state=poisoned.into_inner();state.generation=u64::MAX;state});
            state.generation=state.generation.checked_add(1).unwrap_or(u64::MAX);
            acquire(rest,mutation)
        }
        acquire(&ordered,&mut Some(mutation))
    }

    /// The owner must route every mutation of the protected source through this
    /// method. The generation advances before the mutation, even if it fails.
    /// A poisoned or exhausted lease refuses further snapshots and mutations.
    pub fn mutate<T>(&self, mutation: impl FnOnce() -> T) -> Option<T> {
        let mut state = self.state.lock().ok()?;
        state.generation = state.generation.checked_add(1)?;
        Some(mutation())
    }

    /// For acquisition paths which must not discard data if the lease fails:
    /// perform the mutation exactly once, but permanently disable new permits
    /// when the generation is exhausted or the mutex is poisoned.
    pub fn mutate_always<T>(&self, mutation: impl FnOnce() -> T) -> T {
        match self.state.lock() {
            Ok(mut state) => {
                state.generation = state.generation.checked_add(1).unwrap_or(u64::MAX);
                mutation()
            }
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.generation = u64::MAX;
                mutation()
            }
        }
    }

    /// Permanently reject permits when the owner can no longer publish a
    /// potentially state-changing event. Acquisition may continue, but the
    /// source cannot be trusted again without recreating its authority.
    pub fn invalidate_permanently(&self) {
        match self.state.lock() {
            Ok(mut state) => state.generation = u64::MAX,
            Err(poisoned) => poisoned.into_inner().generation = u64::MAX,
        }
    }

    /// Block permits throughout work performed outside the mutex, including
    /// GUI interaction or I/O. Owners must publish the resulting state before
    /// dropping the guard. Nested and overlapping intervals are supported.
    pub fn begin_update(&self) -> SourceUpdate {
        let active=match self.state.lock() {
            Ok(mut state) => {
                if state.generation==u64::MAX || state.active_updates==usize::MAX {state.generation=u64::MAX;false}
                else {state.generation=state.generation.saturating_add(1);state.active_updates+=1;true}
            }
            Err(poisoned)=>{poisoned.into_inner().generation=u64::MAX;false}
        };
        SourceUpdate{lease:self.clone(),active}
    }

    /// Build the actual outbound source snapshot inside the same boundary that
    /// protects mutations. Never use this for uncoordinated external writers.
    pub fn snapshot<T>(&self, read: impl FnOnce() -> T) -> Option<(T, SourcePermit)> {
        let state = self.state.lock().ok()?;
        if state.generation == u64::MAX || state.active_updates!=0 { return None; }
        let value = read();
        Some((value, SourcePermit { lease: self.clone(), generation: state.generation }))
    }
}

impl Drop for SourceUpdate {
    fn drop(&mut self) {
        if !self.active {return;}
        match self.lease.state.lock() {
            Ok(mut state)=>{
                state.active_updates=state.active_updates.saturating_sub(1);
                state.generation=if std::thread::panicking(){u64::MAX}else{state.generation.saturating_add(1)};
            }
            Err(poisoned)=>{poisoned.into_inner().generation=u64::MAX;}
        }
    }
}

impl SourcePermit {
    pub fn generation(&self) -> u64 { self.generation }
    /// Hold all distinct source locks in address order through one decision.
    /// Duplicate leases are allowed only when captured at the same generation.
    pub(crate) fn commit_all(permits:&[Self], commit:impl FnOnce()->bool)->bool {
        if permits.is_empty(){return false;}
        let mut ordered:Vec<&Self>=permits.iter().collect();
        ordered.sort_by_key(|permit|std::sync::Arc::as_ptr(&permit.lease.state) as usize);
        for pair in ordered.windows(2) {
            if std::sync::Arc::ptr_eq(&pair[0].lease.state,&pair[1].lease.state)
                && pair[0].generation!=pair[1].generation {return false;}
        }
        ordered.dedup_by(|a,b|std::sync::Arc::ptr_eq(&a.lease.state,&b.lease.state));
        fn lock_next<F:FnOnce()->bool>(ordered:&[&SourcePermit],action:&mut Option<F>)->bool {
            let Some((first,rest))=ordered.split_first() else {return action.take().is_some_and(|f|f());};
            let Ok(state)=first.lease.state.lock() else{return false;};
            if state.generation==u64::MAX || state.generation!=first.generation || state.active_updates!=0 {return false;}
            lock_next(rest,action)
        }
        lock_next(&ordered,&mut Some(commit))
    }
    /// Revalidate a captured value and construct its immutable request while
    /// the generation is unchanged. This does not advance the generation.
    pub fn recapture<T>(&self, build:impl FnOnce()->T)->Option<(T,Self)> {
        let state=self.lease.state.lock().ok()?;
        if state.generation==u64::MAX || state.generation!=self.generation || state.active_updates!=0 {return None;}
        Some((build(),self.clone()))
    }
    /// Commit a value prepared outside the lock only if its source snapshot is
    /// still current. The successful commit itself advances the generation.
    /// Callers must not perform filesystem, network or UI work inside `change`.
    pub fn commit_prepared<T>(&self, change: impl FnOnce() -> T) -> Option<T> {
        let mut state = self.lease.state.lock().ok()?;
        if state.generation == u64::MAX || state.generation != self.generation || state.active_updates!=0 { return None; }
        state.generation = state.generation.checked_add(1)?;
        Some(change())
    }
    /// Commit while holding the source boundary; false includes poisoned locks,
    /// changed generations and a failed one-shot dispatch transition.
    pub(crate) fn commit(&self, commit: impl FnOnce() -> bool) -> bool {
        let Ok(state) = self.lease.state.lock() else { return false; };
        state.generation != u64::MAX && state.generation == self.generation && state.active_updates==0 && commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiple_sources_require_every_current_generation() {
        let a=SourceLease::default();let b=SourceLease::default();
        let (_,pa)=a.snapshot(||()).unwrap();let (_,pb)=b.snapshot(||()).unwrap();
        assert!(SourcePermit::commit_all(&[pa.clone(),pb.clone()],||true));
        b.mutate_always(||());
        assert!(!SourcePermit::commit_all(&[pa,pb],||true));
    }
    #[test]
    fn duplicate_source_generations_are_deduplicated_but_mismatches_fail() {
        let lease=SourceLease::default();let (_,a)=lease.snapshot(||()).unwrap();
        assert!(SourcePermit::commit_all(&[a.clone(),a.clone()],||true));
        lease.mutate_always(||());let (_,b)=lease.snapshot(||()).unwrap();
        assert!(!SourcePermit::commit_all(&[a,b],||true));
    }
    #[test]
    fn concurrent_multi_source_commits_use_stable_lock_order() {
        let a=SourceLease::default();let b=SourceLease::default();
        let (_,pa)=a.snapshot(||()).unwrap();let (_,pb)=b.snapshot(||()).unwrap();
        let left=vec![pa.clone(),pb.clone()];let right=vec![pb,pa];
        let worker=std::thread::spawn(move||SourcePermit::commit_all(&left,||true));
        assert!(SourcePermit::commit_all(&right,||true));assert!(worker.join().unwrap());
    }
    #[test]
    fn concurrent_pair_publication_and_dispatch_share_lock_order() {
        let a=SourceLease::default();let b=SourceLease::default();
        let (_,pa)=a.snapshot(||()).unwrap();let (_,pb)=b.snapshot(||()).unwrap();
        let aa=a.clone();let bb=b.clone();
        let writer=std::thread::spawn(move||SourceLease::mutate_pair_always(&bb,&aa,||()));
        let _=SourcePermit::commit_all(&[pa,pb],||true);
        writer.join().unwrap();
    }
    #[test]
    fn triple_mutation_and_dispatch_share_order_and_invalidate_all() {
        let a=SourceLease::default();let b=SourceLease::default();let c=SourceLease::default();
        let (_,pa)=a.snapshot(||()).unwrap();let (_,pb)=b.snapshot(||()).unwrap();let (_,pc)=c.snapshot(||()).unwrap();
        let aa=a.clone();let bb=b.clone();let cc=c.clone();
        let writer=std::thread::spawn(move||SourceLease::mutate_three_always(&cc,&aa,&bb,||()));
        let _=SourcePermit::commit_all(&[pb.clone(),pc.clone(),pa.clone()],||true);
        writer.join().unwrap();
        assert!(!pa.commit(||true));assert!(!pb.commit(||true));assert!(!pc.commit(||true));
        SourceLease::mutate_three_always(&a,&a,&a,||());
        assert!(a.snapshot(||()).is_some());
    }
    #[test]
    fn nonlocking_update_blocks_old_and_new_permits_until_every_guard_drops() {
        let lease=SourceLease::default();let (_,old)=lease.snapshot(||()).unwrap();
        let first=lease.begin_update();let second=lease.begin_update();
        assert!(!old.commit(||true));assert!(lease.snapshot(||()).is_none());
        drop(first);assert!(lease.snapshot(||()).is_none());drop(second);
        assert!(lease.snapshot(||()).is_some());assert!(!old.commit(||true));
    }
    #[test]
    fn panicking_update_permanently_blocks_permits() {
        let lease=SourceLease::default();let (_,old)=lease.snapshot(||()).unwrap();
        let _=std::panic::catch_unwind(|| {let _update=lease.begin_update();panic!("fixture");});
        assert!(lease.snapshot(||()).is_none());assert!(!old.commit(||true));
    }
    #[test]
    fn concurrent_update_completion_cannot_reauthorize_old_permit() {
        let lease=SourceLease::default();let (_,old)=lease.snapshot(||()).unwrap();
        let (entered_tx,entered_rx)=std::sync::mpsc::channel();let (release_tx,release_rx)=std::sync::mpsc::channel();
        let other=lease.clone();let worker=std::thread::spawn(move||{
            let update=other.begin_update();entered_tx.send(()).unwrap();release_rx.recv().unwrap();drop(update);
        });
        entered_rx.recv().unwrap();assert!(lease.snapshot(||()).is_none());assert!(!old.commit(||true));
        release_tx.send(()).unwrap();worker.join().unwrap();
        assert!(!old.commit(||true));assert!(lease.snapshot(||()).is_some());
    }
    #[test]
    fn mutation_invalidates_even_when_data_changes_back() {
        let lease = SourceLease::default();
        let (value, permit) = lease.snapshot(|| 5).unwrap();
        assert_eq!(value, 5);
        lease.mutate(|| ()).unwrap();
        lease.mutate(|| ()).unwrap();
        assert!(!permit.commit(|| true));
    }
    #[test]
    fn current_permit_commits_and_failed_commit_is_not_accepted() {
        let lease = SourceLease::default();
        let (_, permit) = lease.snapshot(|| ()).unwrap();
        assert!(!permit.commit(|| false));
        assert!(permit.commit(|| true));
    }
    #[test]
    fn exhausted_lease_preserves_mutation_but_refuses_new_permits() {
        let lease = SourceLease::default();
        lease.state.lock().unwrap().generation = u64::MAX;
        assert_eq!(lease.mutate_always(|| 7), 7);
        assert!(lease.snapshot(|| ()).is_none());
    }
    #[test]
    fn dispatch_cannot_commit_during_source_mutation() {
        let lease = SourceLease::default();
        let (_, permit) = lease.snapshot(|| ()).unwrap();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let mutation_lease = lease.clone();
        let mutation = std::thread::spawn(move || mutation_lease.mutate_always(|| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }));
        entered_rx.recv().unwrap();
        let commit = std::thread::spawn(move || permit.commit(|| true));
        release_tx.send(()).unwrap();
        mutation.join().unwrap();
        assert!(!commit.join().unwrap());
    }
    #[test]
    fn prepared_result_is_rejected_after_intervening_mutation() {
        let lease = SourceLease::default();
        let (_, permit) = lease.snapshot(|| ()).unwrap();
        lease.mutate_always(|| ());
        let mut applied = false;
        assert!(permit.commit_prepared(|| { applied = true; }).is_none());
        assert!(!applied);
    }
    #[test]
    fn prepared_commit_invalidates_all_previous_permits() {
        let lease = SourceLease::default();
        let (_, first) = lease.snapshot(|| ()).unwrap();
        let (_, second) = lease.snapshot(|| ()).unwrap();
        assert_eq!(first.commit_prepared(|| 7), Some(7));
        assert!(!second.commit(|| true));
    }
}
