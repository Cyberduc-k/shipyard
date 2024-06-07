use crate::{
    error, ARef, AllStorages, AllStoragesView, AllStoragesViewMut, Borrow, SharedBorrow,
    TrackingTimestamp, World,
};

/// [`Borrow`] with state.
pub trait StatefulBorrow {
    #[allow(missing_docs)]
    type View<'a>;
    #[allow(missing_docs)]
    type State: Default + Send + Sync + 'static;

    /// This function is where the actual borrowing happens.
    fn borrow<'a>(
        state: &'a mut Self::State,
        all_storages: &'a AllStorages,
        all_borrow: Option<SharedBorrow<'a>>,
        last_run: Option<TrackingTimestamp>,
        current: TrackingTimestamp,
    ) -> Result<Self::View<'a>, error::GetStorage>;
}

impl<B: Borrow> StatefulBorrow for B {
    type View<'a> = B::View<'a>;
    type State = ();

    fn borrow<'a>(
        _state: &'a mut Self::State,
        all_storages: &'a AllStorages,
        all_borrow: Option<SharedBorrow<'a>>,
        last_run: Option<TrackingTimestamp>,
        current: TrackingTimestamp,
    ) -> Result<Self::View<'a>, error::GetStorage> {
        <B as Borrow>::borrow(all_storages, all_borrow, last_run, current)
    }
}

/// Allows a type to be borrowed by [`World::borrow`], [`World::run`] and workloads.
pub trait StatefulWorldBorrow {
    #[allow(missing_docs)]
    type WorldView<'a>;
    #[allow(missing_docs)]
    type State: Default + Send + Sync + 'static;

    /// This function is where the actual borrowing happens.
    fn world_borrow<'a>(
        state: &'a mut Self::State,
        world: &'a World,
        last_run: Option<TrackingTimestamp>,
        current: TrackingTimestamp,
    ) -> Result<Self::WorldView<'a>, error::GetStorage>;
}

impl<T: StatefulBorrow> StatefulWorldBorrow for T {
    type WorldView<'a> = <T as StatefulBorrow>::View<'a>;
    type State = T::State;

    fn world_borrow<'a>(
        state: &'a mut Self::State,
        world: &'a World,
        last_run: Option<TrackingTimestamp>,
        current: TrackingTimestamp,
    ) -> Result<Self::WorldView<'a>, error::GetStorage> {
        let (all_storages, all_borrow) = unsafe {
            ARef::destructure(
                world
                    .all_storages
                    .borrow()
                    .map_err(error::GetStorage::AllStoragesBorrow)?,
            )
        };

        T::borrow(state, all_storages, Some(all_borrow), last_run, current)
    }
}

impl StatefulWorldBorrow for AllStoragesView<'_> {
    type WorldView<'a> = AllStoragesView<'a>;
    type State = ();

    fn world_borrow<'a>(
        _state: &'a mut Self::State,
        world: &'a World,
        _last_run: Option<TrackingTimestamp>,
        _current: TrackingTimestamp,
    ) -> Result<Self::WorldView<'a>, error::GetStorage> {
        world
            .all_storages
            .borrow()
            .map(AllStoragesView)
            .map_err(error::GetStorage::AllStoragesBorrow)
    }
}

impl StatefulWorldBorrow for AllStoragesViewMut<'_> {
    type WorldView<'a> = AllStoragesViewMut<'a>;
    type State = ();

    #[inline]
    fn world_borrow<'a>(
        _state: &'a mut Self::State,
        world: &'a World,
        _last_run: Option<TrackingTimestamp>,
        _current: TrackingTimestamp,
    ) -> Result<Self::WorldView<'a>, error::GetStorage> {
        world
            .all_storages
            .borrow_mut()
            .map(AllStoragesViewMut)
            .map_err(error::GetStorage::AllStoragesBorrow)
    }
}
