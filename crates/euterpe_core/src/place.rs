//! Building a graph directly in its final memory.
//!
//! Graphs can be large: a 2048-tap convolver alone is 34 KB. Building one
//! with `Default::default()` and moving it into place can put several
//! copies on the stack at once, and the Daisy's stack shares its 128 KB of
//! DTCM with everything else. An [`Arena`] is a static, aligned block of
//! memory that a platform builds the graph in, so the graph itself never
//! lives on the stack.
//!
//! Construction still runs `Default::default()`, so keep it cheap: zeroed
//! buffers build in place. Load assets (IRs, tables) in
//! [`Processor::prepare`](crate::Processor::prepare), not in `Default`.

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem::{MaybeUninit, align_of, size_of};
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// Alignment of every [`Arena`]: enough for any graph (and for cache lines).
pub const ALIGN: usize = 32;

/// `BYTES` of static memory that holds one value at a time.
///
/// Declare it as a `static`, take it once with [`Arena::take`], and
/// [`place`](ArenaRef::place) a graph in it:
///
/// ```
/// use euterpe_core::place::Arena;
///
/// #[derive(Default)]
/// struct Graph {
///     history: [f32; 32],
///     gain: f32,
/// }
///
/// static GRAPH: Arena<4096> = Arena::new();
///
/// let mut arena = GRAPH.take().unwrap();
/// let graph = arena.place::<Graph>();
/// assert_eq!(graph.gain, 0.0);
/// assert!(GRAPH.take().is_none(), "an arena is taken once");
/// ```
#[repr(C, align(32))]
pub struct Arena<const BYTES: usize> {
    mem: UnsafeCell<MaybeUninit<[u8; BYTES]>>,
    taken: AtomicBool,
}

// SAFETY: the memory is only reachable through the one `ArenaRef` that
// `take` hands out, guarded by `taken`.
unsafe impl<const BYTES: usize> Sync for Arena<BYTES> {}

impl<const BYTES: usize> Default for Arena<BYTES> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const BYTES: usize> Arena<BYTES> {
    /// An empty arena. `const`, so a `static` arena lives in `.bss` and costs
    /// no flash.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mem: UnsafeCell::new(MaybeUninit::uninit()),
            taken: AtomicBool::new(false),
        }
    }

    /// Exclusive access to the arena, the first time only.
    pub fn take(&'static self) -> Option<ArenaRef<BYTES>> {
        if self.taken.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(ArenaRef {
            mem: self.mem.get().cast(),
            _arena: PhantomData,
        })
    }
}

/// Exclusive access to an [`Arena`]'s memory.
pub struct ArenaRef<const BYTES: usize> {
    mem: *mut u8,
    _arena: PhantomData<&'static mut [u8; BYTES]>,
}

// SAFETY: an `ArenaRef` is the only handle to its memory.
unsafe impl<const BYTES: usize> Send for ArenaRef<BYTES> {}

impl<const BYTES: usize> ArenaRef<BYTES> {
    /// Build `T::default()` in the arena. A `T` that doesn't fit is a
    /// compile error. The value is dropped when the returned guard is.
    pub fn place<T: Default>(&mut self) -> Placed<'_, T> {
        const {
            assert!(size_of::<T>() <= BYTES, "the graph does not fit the arena");
            assert!(align_of::<T>() <= ALIGN, "the graph needs more alignment");
        };
        let ptr = self.mem.cast::<T>();
        // SAFETY: `ptr` is aligned (checked above), points to `BYTES >=
        // size_of::<T>()` bytes this `ArenaRef` has exclusive access to, and
        // nothing else is placed there while the guard borrows `self`.
        unsafe { ptr.write(T::default()) };
        Placed {
            ptr,
            _borrow: PhantomData,
        }
    }

    /// Like [`ArenaRef::place`], for the rest of the program: the value is
    /// never dropped and the arena can't be reused. For platforms whose
    /// `run` never returns.
    pub fn place_forever<T: Default>(self) -> &'static mut T {
        let mut this = self;
        let placed = this.place::<T>();
        let ptr = placed.ptr;
        core::mem::forget(placed);
        // SAFETY: the arena is `'static` and this `ArenaRef`, its only
        // handle, is consumed here, so the value is never moved, dropped or
        // aliased.
        unsafe { &mut *ptr }
    }
}

/// A value living in an arena. Dropping it drops the value.
pub struct Placed<'a, T> {
    ptr: *mut T,
    _borrow: PhantomData<&'a mut T>,
}

impl<T> Deref for Placed<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: initialized by `place` and exclusively borrowed by `self`.
        unsafe { &*self.ptr }
    }
}

impl<T> DerefMut for Placed<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: as in `deref`, and `&mut self` makes it unique.
        unsafe { &mut *self.ptr }
    }
}

impl<T> Drop for Placed<'_, T> {
    fn drop(&mut self) {
        // SAFETY: initialized by `place`, dropped exactly once, here.
        unsafe { self.ptr.drop_in_place() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::AtomicU32;

    static DROPS: AtomicU32 = AtomicU32::new(0);

    #[derive(Default)]
    struct Counted([u64; 4]);

    impl Drop for Counted {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn values_are_dropped_and_the_arena_reused() {
        static A: Arena<64> = Arena::new();
        let mut arena = A.take().unwrap();
        {
            let mut v = arena.place::<Counted>();
            v.0[3] = 7;
            assert_eq!(v.0, [0, 0, 0, 7]);
        }
        assert_eq!(DROPS.load(Ordering::Relaxed), 1);
        let v = arena.place::<Counted>();
        assert_eq!(v.0, [0; 4], "placed fresh, not left over");
        assert_eq!(
            (&raw const *v).cast::<u8>().align_offset(ALIGN),
            0,
            "aligned"
        );
    }

    #[test]
    fn forever_values_stay() {
        static A: Arena<64> = Arena::new();
        let v: &'static mut Counted = A.take().unwrap().place_forever();
        v.0[0] = 1;
        assert!(A.take().is_none());
    }
}
