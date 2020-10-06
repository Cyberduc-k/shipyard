mod add_component;
mod contains;
// #[cfg(feature = "serde1")]
// mod deser;
mod metadata;
pub mod sort;
mod sparse_array;
mod view_add_entity;
mod window;

pub use add_component::AddComponentUnchecked;
pub use contains::Contains;

// #[cfg(feature = "serde1")]
// pub(crate) use deser::SparseSetSerializer;
// #[cfg(feature = "serde1")]
// use hashbrown::HashMap;
// #[cfg(feature = "serde1")]
// pub(crate) use metadata::SerdeInfos;
pub(crate) use metadata::{
    LoosePack, Metadata, Pack, TightPack, BUCKET_SIZE as SHARED_BUCKET_SIZE,
};
pub(crate) use view_add_entity::ViewAddEntity;
pub(crate) use window::FullRawWindowMut;

use crate::error;
// #[cfg(feature = "serde1")]
// use crate::serde_setup::{GlobalDeConfig, GlobalSerConfig, SerConfig};
use crate::storage::EntityId;
use crate::type_id::TypeId;
use crate::unknown_storage::UnknownStorage;
// #[cfg(feature = "serde1")]
// use alloc::borrow::Cow;
#[cfg(all(not(feature = "std"), feature = "serde1"))]
use alloc::string::ToString;
use alloc::vec::Vec;
use core::any::{type_name, Any};
// #[cfg(feature = "serde1")]
// use deser::SparseSetDeserializer;
use sparse_array::SparseArray;

pub(crate) const BUCKET_SIZE: usize = 256 / core::mem::size_of::<usize>();

/// Component storage.
// A sparse array is a data structure with 2 vectors: one sparse, the other dense.
// Only usize can be added. On insertion, the number is pushed into the dense vector
// and sparse[number] is set to dense.len() - 1.
// For all number present in the sparse array, dense[sparse[number]] == number.
// For all other values if set sparse[number] will have any value left there
// and if set dense[sparse[number]] != number.
// We can't be limited to store solely integers, this is why there is a third vector.
// It mimics the dense vector in regard to insertion/deletion.
//
// An entity is shared is self.shared > 0, the sparse index isn't usize::MAX and dense doesn't point back
// Shared components don't qualify for packs

// shared info in only present in sparse
// inserted and modified info is only present in dense
pub struct SparseSet<T> {
    pub(crate) sparse: SparseArray<[EntityId; BUCKET_SIZE]>,
    pub(crate) dense: Vec<EntityId>,
    pub(crate) data: Vec<T>,
    pub(crate) metadata: Metadata<T>,
}

impl<T> SparseSet<T> {
    #[inline]
    pub(crate) fn new() -> Self {
        SparseSet {
            sparse: SparseArray::new(),
            dense: Vec::new(),
            data: Vec::new(),
            metadata: Default::default(),
        }
    }
    #[inline]
    pub(crate) fn full_raw_window_mut(&mut self) -> FullRawWindowMut<'_, T> {
        FullRawWindowMut::new(self)
    }
    /// Returns a slice of all the components in this storage.
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        &self.data
    }
    #[inline]
    pub(crate) fn clone_indices(&self) -> Vec<EntityId> {
        self.dense.clone()
    }
}

impl<T> SparseSet<T> {
    /// Returns `true` if `entity` owns or shares a component in this storage.
    ///
    /// In case it shares a component, returns `true` even if there is no owned component at the end of the shared chain.
    #[inline]
    pub fn contains(&self, entity: EntityId) -> bool {
        self.index_of(entity).is_some()
    }
    /// Returns `true` if `entity` owns a component in this storage.
    #[inline]
    pub fn contains_owned(&self, entity: EntityId) -> bool {
        self.index_of_owned(entity).is_some()
    }
    /// Returns `true` if `entity` shares a component in this storage.  
    ///
    /// Returns `true` even if there is no owned component at the end of the shared chain.
    #[inline]
    pub fn contains_shared(&self, entity: EntityId) -> bool {
        self.shared_id(entity).is_some()
    }
    /// Returns the length of the storage.
    #[inline]
    pub fn len(&self) -> usize {
        self.dense.len()
    }
    /// Returns true if the storage's length is 0.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }
}

impl<T> SparseSet<T> {
    /// Returns the index of `entity`'s owned component in the `dense` and `data` vectors.
    ///
    /// In case `entity` is shared `index_of` will follow the shared chain to find the owned one at the end.  
    /// This index is only valid for this storage and until a modification happens.
    #[inline]
    pub fn index_of(&self, entity: EntityId) -> Option<usize> {
        self.index_of_owned(entity).or_else(|| {
            let sparse_entity = self.sparse.get(entity)?;

            if sparse_entity.is_shared() && sparse_entity.index() == entity.gen() {
                self.metadata
                    .shared
                    .shared_index(entity)
                    .and_then(|id| self.index_of(id))
            } else {
                None
            }
        })
    }
    /// Returns the index of `entity`'s owned component in the `dense` and `data` vectors.  
    /// This index is only valid for this storage and until a modification happens.
    #[inline]
    pub fn index_of_owned(&self, entity: EntityId) -> Option<usize> {
        self.sparse.get(entity).and_then(|sparse_entity| {
            if sparse_entity.is_owned() && entity.gen() == sparse_entity.gen() {
                Some(sparse_entity.uindex())
            } else {
                None
            }
        })
    }
    /// Returns the index of `entity`'s owned component in the `dense` and `data` vectors.  
    /// This index is only valid for this storage and until a modification happens.
    ///
    /// # Safety
    ///
    /// `entity` has to own a component of this type.  
    /// The index is only valid until a modification occurs in the storage.
    #[inline]
    pub unsafe fn index_of_owned_unchecked(&self, entity: EntityId) -> usize {
        self.sparse.get_unchecked(entity).uindex()
    }
    /// Returns the `EntityId` at a given `index`.
    #[inline]
    pub fn try_id_at(&self, index: usize) -> Option<EntityId> {
        self.dense.get(index).copied()
    }
    /// Returns the `EntityId` at a given `index`.  
    /// Unwraps errors.
    #[cfg(feature = "panic")]
    #[cfg_attr(docsrs, doc(cfg(feature = "panic")))]
    #[track_caller]
    #[inline]
    pub fn id_at(&self, index: usize) -> EntityId {
        match self.try_id_at(index) {
            Some(id) => id,
            None => panic!(
                "Storage has {} components but trying to access the id at index {}.",
                self.len(),
                index
            ),
        }
    }
    #[inline]
    pub(crate) fn get(&self, entity: EntityId) -> Option<&T> {
        self.index_of(entity)
            .map(|index| unsafe { self.data.get_unchecked(index) })
    }
    #[inline]
    pub(crate) fn get_mut(&mut self, entity: EntityId) -> Option<&mut T> {
        let index = self.index_of(entity)?;

        if self.metadata.update.is_some() {
            unsafe {
                let dense_entity = self.dense.get_unchecked_mut(index);

                if !dense_entity.is_inserted() {
                    dense_entity.set_modified();
                }
            }
        }

        Some(unsafe { self.data.get_unchecked_mut(index) })
    }
    /// Returns the `EntityId` `shared` entity points to.
    ///
    /// Returns `None` if the entity isn't shared.
    #[inline]
    pub fn shared_id(&self, shared: EntityId) -> Option<EntityId> {
        match self.sparse.get(shared) {
            Some(sparse_entity)
                if sparse_entity.is_shared() && sparse_entity.index() == shared.gen() =>
            {
                self.metadata.shared.shared_index(shared)
            }
            _ => None,
        }
    }
}

impl<T> SparseSet<T> {
    /// Inserts `value` in the `SparseSet`.
    ///
    /// If an `entity` with the same index but a greater generation already has a component of this type, does nothing and returns `None`.
    ///
    /// Returns what was present at its place, one of the following:
    /// - None - no value present, either `entity` never had this component or it was removed/deleted
    /// - Some(OldComponent::Owned) - `entity` already had this component, it is no replaced
    /// - Some(OldComponent::OldGenOwned) - `entity` didn't have a component but an entity with the same index did and it wasn't removed with the entity
    /// - Some(OldComponent::Shared) - `entity` shared a component
    /// - Some(OldComponent::OldShared) - `entity` didn't have a component but an entity with the same index shared one and it wasn't removed with the entity
    ///
    /// # Update pack
    ///
    /// In case `entity` had a component of this type, the new component will be considered `modified`.
    /// In all other cases it'll be considered `inserted`.
    pub(crate) fn insert(&mut self, value: T, mut entity: EntityId) -> Option<OldComponent<T>> {
        self.sparse.allocate_at(entity);

        // at this point there can't be nothing at the sparse index
        let sparse_entity = unsafe { self.sparse.get_mut_unchecked(entity) };

        let old_component;

        if sparse_entity.is_dead() {
            *sparse_entity =
                EntityId::new_from_parts(self.dense.len() as u64, entity.gen() as u16, 0);

            if self.metadata.update.is_some() {
                entity.set_inserted();
            } else {
                entity.clear_meta();
            }

            self.dense.push(entity);
            self.data.push(value);

            old_component = None;
        } else if sparse_entity.is_owned() {
            if entity.gen() >= sparse_entity.gen() {
                let old_data = unsafe {
                    core::mem::replace(self.data.get_unchecked_mut(sparse_entity.uindex()), value)
                };

                if entity.gen() == sparse_entity.gen() {
                    old_component = Some(OldComponent::Owned(old_data));
                } else {
                    old_component = Some(OldComponent::OldGenOwned(old_data));
                }

                sparse_entity.copy_gen(entity);

                let dense_entity = unsafe { self.dense.get_unchecked_mut(sparse_entity.uindex()) };

                if self.metadata.update.is_some() && !dense_entity.is_inserted() {
                    dense_entity.set_modified();
                }

                dense_entity.copy_index_gen(entity);
            } else {
                old_component = None;
            }
        } else if entity.gen() >= sparse_entity.index() {
            if entity.gen() == sparse_entity.index() {
                old_component = Some(OldComponent::Shared);
            } else {
                old_component = Some(OldComponent::OldGenShared);
            }

            unsafe {
                self.metadata
                    .shared
                    .set_sparse_index_unchecked(entity, EntityId::dead());
            }

            *sparse_entity =
                EntityId::new_from_parts(self.dense.len() as u64, entity.gen() as u16, 0);

            if self.metadata.update.is_some() {
                entity.set_inserted();
            } else {
                entity.clear_meta();
            }

            self.dense.push(entity);
            self.data.push(value);
        } else {
            old_component = None;
        }

        old_component
    }
}

impl<T> SparseSet<T> {
    /// Removes `entity`'s component from this storage.
    ///
    /// ### Errors
    ///
    /// - Storage is tightly or loosly packed.
    #[inline]
    pub fn try_remove(&mut self, entity: EntityId) -> Result<Option<OldComponent<T>>, error::Remove>
    where
        T: 'static,
    {
        if self.metadata.observer_types.is_empty() {
            match self.metadata.pack {
                Pack::Tight(_) => Err(error::Remove::MissingPackStorage(type_name::<T>())),
                Pack::Loose(_) => Err(error::Remove::MissingPackStorage(type_name::<T>())),
                Pack::None => {
                    let component = self.actual_remove(entity);

                    if let Some(update) = &mut self.metadata.update {
                        if let Some(OldComponent::Owned(_)) = &component {
                            update.removed.push(entity);
                        }
                    }

                    Ok(component)
                }
            }
        } else {
            Err(error::Remove::MissingPackStorage(type_name::<T>()))
        }
    }
    /// Removes `entity`'s component from this storage.  
    /// Unwraps errors.
    ///
    /// ### Errors
    ///
    /// - Storage is tightly or loosly packed.
    #[cfg(feature = "panic")]
    #[cfg_attr(docsrs, doc(cfg(feature = "panic")))]
    #[track_caller]
    #[inline]
    pub fn remove(&mut self, entity: EntityId) -> Option<OldComponent<T>>
    where
        T: 'static,
    {
        match self.try_remove(entity) {
            Ok(old_component) => old_component,
            Err(err) => panic!("{:?}", err),
        }
    }
    pub(crate) fn actual_remove(&mut self, entity: EntityId) -> Option<OldComponent<T>> {
        let mut sparse_entity = self.sparse.get(entity)?;

        if sparse_entity.is_owned() && entity.gen() >= sparse_entity.gen() {
            unsafe {
                *self.sparse.get_mut_unchecked(entity) = EntityId::dead();
            }

            match &mut self.metadata.pack {
                Pack::Tight(tight) => {
                    if sparse_entity.uindex() < tight.len {
                        tight.len -= 1;

                        unsafe {
                            self.sparse
                                .get_mut_unchecked(*self.dense.get_unchecked(tight.len))
                                .copy_index(sparse_entity);
                        }

                        self.dense.swap(sparse_entity.uindex(), tight.len);
                        self.data.swap(sparse_entity.uindex(), tight.len);

                        sparse_entity.set_index(tight.len as u64);
                    }
                }
                Pack::Loose(loose) => {
                    if sparse_entity.uindex() < loose.len {
                        loose.len -= 1;

                        unsafe {
                            self.sparse
                                .get_mut_unchecked(*self.dense.get_unchecked(loose.len))
                                .copy_index(sparse_entity);
                        }

                        self.dense.swap(sparse_entity.uindex(), loose.len);
                        self.data.swap(sparse_entity.uindex(), loose.len);

                        sparse_entity.set_index(loose.len as u64);
                    }
                }
                _ => {}
            }

            self.dense.swap_remove(sparse_entity.uindex());
            let component = self.data.swap_remove(sparse_entity.uindex());

            unsafe {
                let last = *self.dense.get_unchecked(sparse_entity.uindex());
                self.sparse
                    .get_mut_unchecked(last)
                    .copy_index(sparse_entity);
            }

            if entity.gen() == sparse_entity.gen() {
                Some(OldComponent::Owned(component))
            } else {
                Some(OldComponent::OldGenOwned(component))
            }
        } else if sparse_entity.is_shared() && entity.gen() >= sparse_entity.index() {
            unsafe {
                *self.sparse.get_mut_unchecked(entity) = EntityId::dead();

                self.metadata
                    .shared
                    .set_sparse_index_unchecked(entity, EntityId::dead());
            }

            if entity.gen() == sparse_entity.index() {
                Some(OldComponent::Shared)
            } else {
                Some(OldComponent::OldGenShared)
            }
        } else {
            None
        }
    }
    /// Deletes `entity`'s component from this storage.
    ///
    /// ### Errors
    ///
    /// - Storage is tightly or loosly packed.
    #[inline]
    pub fn try_delete(&mut self, entity: EntityId) -> Result<(), error::Remove>
    where
        T: 'static,
    {
        if self.metadata.observer_types.is_empty() {
            match self.metadata.pack {
                Pack::Tight(_) => Err(error::Remove::MissingPackStorage(type_name::<T>())),
                Pack::Loose(_) => Err(error::Remove::MissingPackStorage(type_name::<T>())),
                _ => {
                    self.actual_delete(entity);
                    Ok(())
                }
            }
        } else {
            Err(error::Remove::MissingPackStorage(type_name::<T>()))
        }
    }
    /// Deletes `entity`'s component from this storage.  
    /// Unwraps errors.
    ///
    /// ### Errors
    ///
    /// - Storage is tightly or loosly packed.
    #[cfg(feature = "panic")]
    #[cfg_attr(docsrs, doc(cfg(feature = "panic")))]
    #[track_caller]
    #[inline]
    pub fn delete(&mut self, entity: EntityId)
    where
        T: 'static,
    {
        match self.try_delete(entity) {
            Ok(_) => (),
            Err(err) => panic!("{:?}", err),
        }
    }
    #[inline]
    pub(crate) fn actual_delete(&mut self, entity: EntityId) {
        if let Some(OldComponent::Owned(component)) = self.actual_remove(entity) {
            if let Some(update) = &mut self.metadata.update {
                update.deleted.push((entity, component));
            }
        }
    }
}

impl<T> SparseSet<T> {
    /// Returns the *deleted* components of an update packed storage.
    ///
    /// ### Errors
    ///
    /// - Storage isn't update packed.
    #[inline]
    pub fn try_deleted(&self) -> Result<&[(EntityId, T)], error::NotUpdatePack> {
        if let Some(update) = &self.metadata.update {
            Ok(&update.deleted)
        } else {
            Err(error::NotUpdatePack)
        }
    }
    /// Returns the *deleted* components of an update packed storage.  
    /// Unwraps errors.
    ///
    /// ### Errors
    ///
    /// - Storage isn't update packed.
    #[cfg(feature = "panic")]
    #[cfg_attr(docsrs, doc(cfg(feature = "panic")))]
    #[track_caller]
    #[inline]
    pub fn deleted(&self) -> &[(EntityId, T)] {
        match self.try_deleted() {
            Ok(deleted) => deleted,
            Err(err) => panic!("{:?}", err),
        }
    }
    /// Returns the ids of *removed* components of an update packed storage.
    ///
    /// ### Errors
    ///
    /// - Storage isn't update packed.
    #[inline]
    pub fn try_removed(&self) -> Result<&[EntityId], error::NotUpdatePack> {
        if let Some(update) = &self.metadata.update {
            Ok(&update.removed)
        } else {
            Err(error::NotUpdatePack)
        }
    }
    /// Returns the ids of *removed* components of an update packed storage.
    /// Unwraps errors.
    ///
    /// ### Errors
    ///
    /// - Storage isn't update packed.
    #[cfg(feature = "panic")]
    #[cfg_attr(docsrs, doc(cfg(feature = "panic")))]
    #[track_caller]
    #[inline]
    pub fn removed(&self) -> &[EntityId] {
        match self.try_removed() {
            Ok(removed) => removed,
            Err(err) => panic!("{:?}", err),
        }
    }
    /// Takes ownership of the *deleted* components of an update packed storage.
    ///
    /// ### Errors
    ///
    /// - Storage isn't update packed.
    #[inline]
    pub fn try_take_deleted(&mut self) -> Result<Vec<(EntityId, T)>, error::NotUpdatePack> {
        if let Some(update) = &mut self.metadata.update {
            let mut vec = Vec::with_capacity(update.deleted.capacity());

            core::mem::swap(&mut vec, &mut update.deleted);

            Ok(vec)
        } else {
            Err(error::NotUpdatePack)
        }
    }
    /// Takes ownership of the *deleted* components of an update packed storage.  
    /// Unwraps errors.
    ///
    /// ### Errors
    ///
    /// - Storage isn't update packed.
    #[cfg(feature = "panic")]
    #[cfg_attr(docsrs, doc(cfg(feature = "panic")))]
    #[track_caller]
    #[inline]
    pub fn take_deleted(&mut self) -> Vec<(EntityId, T)> {
        match self.try_take_deleted() {
            Ok(deleted) => deleted,
            Err(err) => panic!("{:?}", err),
        }
    }
    /// Takes ownership of the ids of *removed* components of an update packed storage.
    ///
    /// ### Errors
    ///
    /// - Storage isn't update packed.
    #[inline]
    pub fn try_take_removed(&mut self) -> Result<Vec<EntityId>, error::NotUpdatePack> {
        if let Some(update) = &mut self.metadata.update {
            let mut vec = Vec::with_capacity(update.removed.capacity());

            core::mem::swap(&mut vec, &mut update.removed);

            Ok(vec)
        } else {
            Err(error::NotUpdatePack)
        }
    }
    /// Takes ownership of the ids of *removed* components of an update packed storage.  
    /// Unwraps errors.
    ///
    /// ### Errors
    ///
    /// - Storage isn't update packed.
    #[cfg(feature = "panic")]
    #[cfg_attr(docsrs, doc(cfg(feature = "panic")))]
    #[track_caller]
    #[inline]
    pub fn take_removed(&mut self) -> Vec<EntityId> {
        match self.try_take_removed() {
            Ok(removed) => removed,
            Err(err) => panic!("{:?}", err),
        }
    }
    /// Moves all component in the *inserted* section of an update packed storage to the *neutral* section.
    ///
    /// ### Errors
    ///
    /// - Storage isn't update packed.
    #[inline]
    pub fn try_clear_inserted(&mut self) -> Result<(), error::NotUpdatePack> {
        if self.metadata.update.is_some() {
            for id in &mut *self.dense {
                if id.is_inserted() {
                    id.clear_meta();
                }
            }

            Ok(())
        } else {
            Err(error::NotUpdatePack)
        }
    }
    /// Moves all component in the *inserted* section of an update packed storage to the *neutral* section.  
    /// Unwraps errors.
    ///
    /// ### Errors
    ///
    /// - Storage isn't update packed.
    #[cfg(feature = "panic")]
    #[cfg_attr(docsrs, doc(cfg(feature = "panic")))]
    #[track_caller]
    #[inline]
    pub fn clear_inserted(&mut self) {
        match self.try_clear_inserted() {
            Ok(_) => (),
            Err(err) => panic!("{:?}", err),
        }
    }
    /// Moves all component in the *modified* section of an update packed storage to the *neutral* section.
    ///
    /// ### Errors
    ///
    /// - Storage isn't update packed.
    #[inline]
    pub fn try_clear_modified(&mut self) -> Result<(), error::NotUpdatePack> {
        if self.metadata.update.is_some() {
            for id in &mut *self.dense {
                if id.is_modified() {
                    id.clear_meta();
                }
            }

            Ok(())
        } else {
            Err(error::NotUpdatePack)
        }
    }
    /// Moves all component in the *modified* section of an update packed storage to the *neutral* section.  
    /// Unwraps errors.
    ///
    /// ### Errors
    ///
    /// - Storage isn't update packed.
    #[cfg(feature = "panic")]
    #[cfg_attr(docsrs, doc(cfg(feature = "panic")))]
    #[track_caller]
    #[inline]
    pub fn clear_modified(&mut self) {
        match self.try_clear_modified() {
            Ok(_) => (),
            Err(err) => panic!("{:?}", err),
        }
    }
    /// Moves all component in the *inserted* and *modified* section of an update packed storage to the *neutral* section.
    ///
    /// ### Errors
    ///
    /// - Storage isn't update packed.
    #[inline]
    pub fn try_clear_inserted_and_modified(&mut self) -> Result<(), error::NotUpdatePack> {
        if self.metadata.update.is_some() {
            for id in &mut *self.dense {
                id.clear_meta();
            }

            Ok(())
        } else {
            Err(error::NotUpdatePack)
        }
    }
    /// Moves all component in the *inserted* and *modified* section of an update packed storage to the *neutral* section.  
    /// Unwraps errors.
    ///
    /// ### Errors
    ///
    /// - Storage isn't update packed.
    #[cfg(feature = "panic")]
    #[cfg_attr(docsrs, doc(cfg(feature = "panic")))]
    #[track_caller]
    #[inline]
    pub fn clear_inserted_and_modified(&mut self) {
        match self.try_clear_inserted_and_modified() {
            Ok(_) => (),
            Err(err) => panic!("{:?}", err),
        }
    }
    //          ▼ old end of pack
    //              ▼ new end of pack
    // [_ _ _ _ | _ | _ _ _ _ _]
    //            ▲       ▼
    //            ---------
    //              pack
    pub(crate) fn pack(&mut self, entity: EntityId) {
        if let Some(sparse_entity) = self.sparse.get(entity) {
            match &mut self.metadata.pack {
                Pack::Tight(tight) => {
                    if sparse_entity.uindex() >= tight.len {
                        unsafe {
                            self.sparse
                                .get_mut_unchecked(entity)
                                .set_index(tight.len as u64);
                            self.sparse
                                .get_mut_unchecked(*self.dense.get_unchecked(tight.len))
                                .copy_index(sparse_entity);
                        }

                        self.dense.swap(tight.len, sparse_entity.uindex());
                        self.data.swap(tight.len, sparse_entity.uindex());

                        tight.len += 1;
                    }
                }
                Pack::Loose(loose) => {
                    if sparse_entity.uindex() >= loose.len {
                        unsafe {
                            self.sparse
                                .get_mut_unchecked(entity)
                                .set_index(loose.len as u64);
                            self.sparse
                                .get_mut_unchecked(*self.dense.get_unchecked(loose.len))
                                .copy_index(sparse_entity);
                        }

                        self.dense.swap(loose.len, sparse_entity.uindex());
                        self.data.swap(loose.len, sparse_entity.uindex());

                        loose.len += 1;
                    }
                }
                _ => {}
            }
        }
    }
    pub(crate) fn unpack(&mut self, entity: EntityId) {
        if let Some(sparse_entity) = self.sparse.get(entity) {
            match &mut self.metadata.pack {
                Pack::Tight(tight) => {
                    if sparse_entity.uindex() < tight.len {
                        tight.len -= 1;

                        self.dense.swap(sparse_entity.uindex(), tight.len);
                        self.data.swap(sparse_entity.uindex(), tight.len);

                        unsafe {
                            self.sparse
                                .get_mut_unchecked(
                                    *self.dense.get_unchecked(sparse_entity.uindex()),
                                )
                                .copy_index(sparse_entity);
                            self.sparse
                                .get_mut_unchecked(*self.dense.get_unchecked(tight.len))
                                .set_index(tight.len as u64);
                        }
                    }
                }
                Pack::Loose(loose) => {
                    if sparse_entity.uindex() < loose.len {
                        loose.len -= 1;

                        self.dense.swap(sparse_entity.uindex(), loose.len);
                        self.data.swap(sparse_entity.uindex(), loose.len);

                        unsafe {
                            self.sparse
                                .get_mut_unchecked(
                                    *self.dense.get_unchecked(sparse_entity.uindex()),
                                )
                                .copy_index(sparse_entity);
                            self.sparse
                                .get_mut_unchecked(*self.dense.get_unchecked(loose.len))
                                .set_index(loose.len as u64);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    /// Update packs this storage making it track *inserted*, *modified*, *removed* and *deleted* components.  
    /// Does nothing if the storage is already update packed.
    #[inline]
    pub fn update_pack(&mut self) {
        self.metadata.update.get_or_insert_with(Default::default);
    }
}

impl<T> SparseSet<T> {
    /// Reserves memory for at least `additional` components. Adding components can still allocate though.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.dense.reserve(additional);
        self.data.reserve(additional);
    }
    /// Deletes all components in this storage.
    pub fn clear(&mut self) {
        for &id in &self.dense {
            unsafe {
                *self.sparse.get_mut_unchecked(id) = EntityId::dead();
            }
        }
        match &mut self.metadata.pack {
            Pack::Tight(tight) => tight.len = 0,
            Pack::Loose(loose) => loose.len = 0,
            Pack::None => {}
        }

        if let Some(update) = &mut self.metadata.update {
            update
                .deleted
                .extend(self.dense.drain(..).zip(self.data.drain(..)));
        }

        self.dense.clear();
        self.data.clear();
    }
    /// Shares `owned`'s component with `shared` entity.  
    /// Deleting `owned`'s component won't stop the sharing.  
    /// Trying to share an entity with itself won't do anything.
    ///
    /// ### Errors
    ///
    /// - `entity` already had a owned component of this type.
    #[inline]
    pub fn try_share(&mut self, owned: EntityId, shared: EntityId) -> Result<(), error::Share> {
        if owned != shared {
            self.sparse.allocate_at(shared);

            if !self.contains_owned(shared) {
                unsafe {
                    *self.sparse.get_mut_unchecked(shared) = EntityId::new_shared(shared);

                    self.metadata
                        .shared
                        .set_sparse_index_unchecked(shared, owned);
                }

                Ok(())
            } else {
                Err(error::Share)
            }
        } else {
            Ok(())
        }
    }
    /// Shares `owned`'s component with `shared` entity.  
    /// Deleting `owned`'s component won't stop the sharing.  
    /// Trying to share an entity with itself won't do anything.  
    /// Unwraps errors.
    ///
    /// ### Errors
    ///
    /// - `entity` already had a owned component of this type.
    #[cfg(feature = "panic")]
    #[cfg_attr(docsrs, doc(cfg(feature = "panic")))]
    #[inline]
    pub fn share(&mut self, owned: EntityId, shared: EntityId) {
        match self.try_share(owned, shared) {
            Ok(_) => (),
            Err(err) => panic!("{:?}", err),
        }
    }
    /// Makes `entity` stop observing another entity.
    ///
    /// ### Errors
    ///
    /// - `entity` was not observing any entity.
    #[inline]
    pub fn try_unshare(&mut self, entity: EntityId) -> Result<(), error::Unshare> {
        if self.contains_shared(entity) {
            unsafe {
                *self.sparse.get_mut_unchecked(entity) = EntityId::dead();
                self.metadata
                    .shared
                    .set_sparse_index_unchecked(entity, EntityId::dead());
            }

            Ok(())
        } else {
            Err(error::Unshare)
        }
    }
    /// Makes `entity` stop observing another entity.  
    /// Unwraps errors.
    ///
    /// ### Errors
    ///
    /// - `entity` was not observing any entity.
    #[cfg(feature = "panic")]
    #[cfg_attr(docsrs, doc(cfg(feature = "panic")))]
    #[inline]
    pub fn unshare(&mut self, entity: EntityId) {
        match self.try_unshare(entity) {
            Ok(_) => (),
            Err(err) => panic!("{:?}", err),
        }
    }
    /// Applies the given function `f` to the entities `a` and `b`.  
    /// The two entities shouldn't point to the same component.
    ///
    /// ### Errors
    ///
    /// - MissingComponent - if one of the entity doesn't have any component in the storage.
    /// - IdenticalIds - if the two entities point to the same component.
    #[inline]
    pub fn try_apply<R, F: FnOnce(&mut T, &T) -> R>(
        &mut self,
        a: EntityId,
        b: EntityId,
        f: F,
    ) -> Result<R, error::Apply> {
        let a_index = self
            .index_of(a)
            .ok_or_else(|| error::Apply::MissingComponent(a))?;
        let b_index = self
            .index_of(b)
            .ok_or_else(|| error::Apply::MissingComponent(b))?;

        if a_index != b_index {
            if self.metadata.update.is_some() {
                unsafe {
                    let a_dense = self.dense.get_unchecked_mut(a_index);

                    if !a_dense.is_inserted() {
                        a_dense.set_modified();
                    }
                }
            }

            let a = unsafe { &mut *self.data.as_mut_ptr().add(a_index) };
            let b = unsafe { &*self.data.as_mut_ptr().add(b_index) };

            Ok(f(a, b))
        } else {
            Err(error::Apply::IdenticalIds)
        }
    }
    /// Applies the given function `f` to the entities `a` and `b`.  
    /// The two entities shouldn't point to the same component.  
    /// Unwraps errors.
    ///
    /// ### Errors
    ///
    /// - MissingComponent - if one of the entity doesn't have any component in the storage.
    /// - IdenticalIds - if the two entities point to the same component.
    #[cfg(feature = "panic")]
    #[cfg_attr(docsrs, doc(cfg(feature = "panic")))]
    #[inline]
    pub fn apply<R, F: FnOnce(&mut T, &T) -> R>(&mut self, a: EntityId, b: EntityId, f: F) -> R {
        match self.try_apply(a, b, f) {
            Ok(result) => result,
            Err(err) => panic!("{:?}", err),
        }
    }
    /// Applies the given function `f` to the entities `a` and `b`.  
    /// The two entities shouldn't point to the same component.
    ///
    /// ### Errors
    ///
    /// - MissingComponent - if one of the entity doesn't have any component in the storage.
    /// - IdenticalIds - if the two entities point to the same component.
    #[inline]
    pub fn try_apply_mut<R, F: FnOnce(&mut T, &mut T) -> R>(
        &mut self,
        a: EntityId,
        b: EntityId,
        f: F,
    ) -> Result<R, error::Apply> {
        let a_index = self
            .index_of(a)
            .ok_or_else(|| error::Apply::MissingComponent(a))?;
        let b_index = self
            .index_of(b)
            .ok_or_else(|| error::Apply::MissingComponent(b))?;

        if a_index != b_index {
            if self.metadata.update.is_some() {
                unsafe {
                    let a_dense = self.dense.get_unchecked_mut(a_index);

                    if !a_dense.is_inserted() {
                        a_dense.set_modified();
                    }

                    let b_dense = self.dense.get_unchecked_mut(b_index);
                    if !b_dense.is_inserted() {
                        b_dense.set_modified();
                    }
                }
            }

            let a = unsafe { &mut *self.data.as_mut_ptr().add(a_index) };
            let b = unsafe { &mut *self.data.as_mut_ptr().add(b_index) };

            Ok(f(a, b))
        } else {
            Err(error::Apply::IdenticalIds)
        }
    }
    /// Applies the given function `f` to the entities `a` and `b`.  
    /// The two entities shouldn't point to the same component.  
    /// Unwraps errors.
    ///
    /// ### Errors
    ///
    /// - MissingComponent - if one of the entity doesn't have any component in the storage.
    /// - IdenticalIds - if the two entities point to the same component.
    #[cfg(feature = "panic")]
    #[cfg_attr(docsrs, doc(cfg(feature = "panic")))]
    #[inline]
    pub fn apply_mut<R, F: FnOnce(&mut T, &mut T) -> R>(
        &mut self,
        a: EntityId,
        b: EntityId,
        f: F,
    ) -> R {
        match self.try_apply_mut(a, b, f) {
            Ok(result) => result,
            Err(err) => panic!("{:?}", err),
        }
    }
}

// #[cfg(feature = "serde1")]
// impl<T: serde::Serialize + for<'de> serde::Deserialize<'de> + 'static> SparseSet<T> {
//     /// Setup serialization for this storage.
//     /// Needs to be called for a storage to be serialized.
//     #[cfg_attr(docsrs, doc(cfg(feature = "panic")))]
//     pub fn setup_serde(&mut self, ser_config: SerConfig) {
//         self.metadata.serde = Some(SerdeInfos::new(ser_config));
//     }
// }

impl<T> core::ops::Index<EntityId> for SparseSet<T> {
    type Output = T;
    #[inline]
    fn index(&self, entity: EntityId) -> &Self::Output {
        self.get(entity).unwrap()
    }
}

impl<T> core::ops::IndexMut<EntityId> for SparseSet<T> {
    #[inline]
    fn index_mut(&mut self, entity: EntityId) -> &mut Self::Output {
        self.get_mut(entity).unwrap()
    }
}

impl<T: 'static> UnknownStorage for SparseSet<T> {
    #[inline]
    fn any(&self) -> &dyn Any {
        self
    }
    #[inline]
    fn any_mut(&mut self) -> &mut dyn Any {
        self
    }
    #[inline]
    fn delete(&mut self, entity: EntityId, storage_to_unpack: &mut Vec<TypeId>) {
        self.actual_delete(entity);

        storage_to_unpack.reserve(self.metadata.observer_types.len());

        let mut i = 0;
        for observer in self.metadata.observer_types.iter().copied() {
            while i < storage_to_unpack.len() && observer < storage_to_unpack[i] {
                i += 1;
            }
            if storage_to_unpack.is_empty() || observer != storage_to_unpack[i] {
                storage_to_unpack.insert(i, observer);
            }
        }
    }
    #[inline]
    fn clear(&mut self) {
        <Self>::clear(self)
    }
    #[inline]
    fn unpack(&mut self, entity: EntityId) {
        Self::unpack(self, entity);
    }
    #[inline]
    fn share(&mut self, owned: EntityId, shared: EntityId) {
        let _ = Self::try_share(self, owned, shared);
    }
    //     #[cfg(feature = "serde1")]
    //     fn should_serialize(&self, _: GlobalSerConfig) -> bool {
    //         self.metadata.serde.is_some()
    //     }
    //     #[cfg(feature = "serde1")]
    //     fn serialize_identifier(&self) -> Cow<'static, str> {
    //         self.metadata
    //             .serde
    //             .as_ref()
    //             .and_then(|serde| serde.identifier.as_ref())
    //             .map(|identifier| identifier.0.clone())
    //             .unwrap_or("".into())
    //     }
    //     #[cfg(feature = "serde1")]
    //     fn serialize(
    //         &self,
    //         ser_config: GlobalSerConfig,
    //         serializer: &mut dyn crate::erased_serde::Serializer,
    //     ) -> crate::erased_serde::Result<crate::erased_serde::Ok> {
    //         (self.metadata.serde.as_ref().unwrap().serialization)(self, ser_config, serializer)
    //     }
    //     #[cfg(feature = "serde1")]
    //     fn deserialize(
    //         &self,
    //     ) -> Option<
    //         fn(
    //             GlobalDeConfig,
    //             &HashMap<EntityId, EntityId>,
    //             &mut dyn crate::erased_serde::Deserializer<'_>,
    //         ) -> Result<crate::storage::Storage, crate::erased_serde::Error>,
    //     > {
    //         Some(self.metadata.serde.as_ref()?.deserialization)
    //     }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum OldComponent<T> {
    Owned(T),
    OldGenOwned(T),
    Shared,
    OldGenShared,
}

impl<T> OldComponent<T> {
    /// Extracts the value inside `OldComponent`.
    #[cfg(feature = "panic")]
    #[cfg_attr(docsrs, doc(cfg(feature = "panic")))]
    #[inline]
    pub fn unwrap_owned(self) -> T {
        match self {
            Self::Owned(component) => component,
            Self::OldGenOwned(_) => {
                panic!("Called `OldComponent::unwrap_owned` on a `OldGenOwned` variant")
            }
            Self::Shared => panic!("Called `OldComponent::unwrap_owned` on a `Shared` variant"),
            Self::OldGenShared => {
                panic!("Called `OldComponent::unwrap_owned` on a `OldShared` variant")
            }
        }
    }
}

#[test]
fn insert() {
    let mut array = SparseSet::new();

    assert!(array
        .insert("0", EntityId::new_from_parts(0, 0, 0))
        .is_none());
    assert_eq!(array.dense, &[EntityId::new_from_parts(0, 0, 0)]);
    assert_eq!(array.data, &["0"]);
    assert_eq!(array.get(EntityId::new_from_parts(0, 0, 0)), Some(&"0"));

    assert!(array
        .insert("1", EntityId::new_from_parts(1, 0, 0))
        .is_none());
    assert_eq!(
        array.dense,
        &[
            EntityId::new_from_parts(0, 0, 0),
            EntityId::new_from_parts(1, 0, 0)
        ]
    );
    assert_eq!(array.data, &["0", "1"]);
    assert_eq!(array.get(EntityId::new_from_parts(0, 0, 0)), Some(&"0"));
    assert_eq!(array.get(EntityId::new_from_parts(1, 0, 0)), Some(&"1"));

    assert!(array
        .insert("5", EntityId::new_from_parts(5, 0, 0))
        .is_none());
    assert_eq!(
        array.dense,
        &[
            EntityId::new_from_parts(0, 0, 0),
            EntityId::new_from_parts(1, 0, 0),
            EntityId::new_from_parts(5, 0, 0)
        ]
    );
    assert_eq!(array.data, &["0", "1", "5"]);
    assert_eq!(
        array.get_mut(EntityId::new_from_parts(5, 0, 0)),
        Some(&mut "5")
    );

    assert_eq!(array.get(EntityId::new_from_parts(4, 0, 0)), None);
}

#[test]
fn remove() {
    let mut array = SparseSet::new();
    array.insert("0", EntityId::new_from_parts(0, 0, 0));
    array.insert("5", EntityId::new_from_parts(5, 0, 0));
    array.insert("10", EntityId::new_from_parts(10, 0, 0));

    assert_eq!(
        array.try_remove(EntityId::new_from_parts(0, 0, 0)),
        Ok(Some(OldComponent::Owned("0")))
    );
    assert_eq!(
        array.dense,
        &[
            EntityId::new_from_parts(10, 0, 0),
            EntityId::new_from_parts(5, 0, 0)
        ]
    );
    assert_eq!(array.data, &["10", "5"]);
    assert_eq!(array.get(EntityId::new_from_parts(0, 0, 0)), None);
    assert_eq!(array.get(EntityId::new_from_parts(5, 0, 0)), Some(&"5"));
    assert_eq!(array.get(EntityId::new_from_parts(10, 0, 0)), Some(&"10"));

    array.insert("3", EntityId::new_from_parts(3, 0, 0));
    array.insert("100", EntityId::new_from_parts(100, 0, 0));
    assert_eq!(
        array.dense,
        &[
            EntityId::new_from_parts(10, 0, 0),
            EntityId::new_from_parts(5, 0, 0),
            EntityId::new_from_parts(3, 0, 0),
            EntityId::new_from_parts(100, 0, 0)
        ]
    );
    assert_eq!(array.data, &["10", "5", "3", "100"]);
    assert_eq!(array.get(EntityId::new_from_parts(0, 0, 0)), None);
    assert_eq!(array.get(EntityId::new_from_parts(3, 0, 0)), Some(&"3"));
    assert_eq!(array.get(EntityId::new_from_parts(5, 0, 0)), Some(&"5"));
    assert_eq!(array.get(EntityId::new_from_parts(10, 0, 0)), Some(&"10"));
    assert_eq!(array.get(EntityId::new_from_parts(100, 0, 0)), Some(&"100"));

    assert_eq!(
        array.try_remove(EntityId::new_from_parts(3, 0, 0)),
        Ok(Some(OldComponent::Owned("3")))
    );
    assert_eq!(
        array.dense,
        &[
            EntityId::new_from_parts(10, 0, 0),
            EntityId::new_from_parts(5, 0, 0),
            EntityId::new_from_parts(100, 0, 0)
        ]
    );
    assert_eq!(array.data, &["10", "5", "100"]);
    assert_eq!(array.get(EntityId::new_from_parts(0, 0, 0)), None);
    assert_eq!(array.get(EntityId::new_from_parts(3, 0, 0)), None);
    assert_eq!(array.get(EntityId::new_from_parts(5, 0, 0)), Some(&"5"));
    assert_eq!(array.get(EntityId::new_from_parts(10, 0, 0)), Some(&"10"));
    assert_eq!(array.get(EntityId::new_from_parts(100, 0, 0)), Some(&"100"));

    assert_eq!(
        array.try_remove(EntityId::new_from_parts(100, 0, 0)),
        Ok(Some(OldComponent::Owned("100")))
    );
    assert_eq!(
        array.dense,
        &[
            EntityId::new_from_parts(10, 0, 0),
            EntityId::new_from_parts(5, 0, 0)
        ]
    );
    assert_eq!(array.data, &["10", "5"]);
    assert_eq!(array.get(EntityId::new_from_parts(0, 0, 0)), None);
    assert_eq!(array.get(EntityId::new_from_parts(3, 0, 0)), None);
    assert_eq!(array.get(EntityId::new_from_parts(5, 0, 0)), Some(&"5"));
    assert_eq!(array.get(EntityId::new_from_parts(10, 0, 0)), Some(&"10"));
    assert_eq!(array.get(EntityId::new_from_parts(100, 0, 0)), None);
}
