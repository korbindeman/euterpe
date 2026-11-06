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
pub struct Arena<const BYTES: usize> {
    mem: Region<BYTES>,
    taken: AtomicBool,
}

/// `BYTES` of aligned, uninitialized memory, for an arena whose memory sits
/// in a `NOLOAD` linker section (the Daisy's AXI SRAM), which startup
/// doesn't zero. Its [`LinkedArena`] keeps the `taken` flag in `.bss`.
///
/// ```
/// use euterpe_core::place::{LinkedArena, Region};
///
/// // On the Daisy: #[unsafe(link_section = ".sram")]
/// static MEM: Region<1024> = Region::new();
/// static ARENA: LinkedArena<1024> = LinkedArena::new(&MEM);
///
/// let v: &'static mut [f32; 32] = ARENA.take().unwrap().place_forever();
/// assert_eq!(v[31], 0.0);
/// ```
#[repr(C, align(32))]
pub struct Region<const BYTES: usize>(UnsafeCell<MaybeUninit<[u8; BYTES]>>);

// SAFETY: a region is only reached through the one `ArenaRef` its arena's
// `take` hands out.
unsafe impl<const BYTES: usize> Sync for Region<BYTES> {}

impl<const BYTES: usize> Region<BYTES> {
    #[must_use]
    pub const fn new() -> Self {
        Self(UnsafeCell::new(MaybeUninit::uninit()))
    }
}

impl<const BYTES: usize> Default for Region<BYTES> {
    fn default() -> Self {
        Self::new()
    }
}

/// An arena over a [`Region`] that lives elsewhere.
pub struct LinkedArena<const BYTES: usize> {
    mem: &'static Region<BYTES>,
    taken: AtomicBool,
}

impl<const BYTES: usize> LinkedArena<BYTES> {
    #[must_use]
    pub const fn new(mem: &'static Region<BYTES>) -> Self {
        Self {
            mem,
            taken: AtomicBool::new(false),
        }
    }

    /// Exclusive access to the region, the first time only.
    pub fn take(&self) -> Option<ArenaRef<BYTES>> {
        take(self.mem, &self.taken)
    }
}

fn take<const BYTES: usize>(
    mem: &'static Region<BYTES>,
    taken: &AtomicBool,
) -> Option<ArenaRef<BYTES>> {
    if taken.swap(true, Ordering::AcqRel) {
        return None;
    }
    Some(ArenaRef {
        mem: mem.0.get().cast(),
        _arena: PhantomData,
    })
}

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
            mem: Region::new(),
            taken: AtomicBool::new(false),
        }
    }

    /// Exclusive access to the arena, the first time only.
    pub fn take(&'static self) -> Option<ArenaRef<BYTES>> {
        take(&self.mem, &self.taken)
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
        self.place_with(Init::DEFAULT)
    }

    /// Like [`place`](Self::place), built the way `init` says.
    pub fn place_with<T: Default>(&mut self, init: Init) -> Placed<'_, T> {
        const {
            assert!(Self::fits::<T>(), "the graph does not fit the arena");
        };
        // SAFETY: `Init::build` initializes the whole value.
        unsafe { self.build_unchecked(|p| init.build(p)) }
    }

    /// Whether a `T` fits: [`place`](Self::place) compiles for it.
    #[must_use]
    pub const fn fits<T>() -> bool {
        size_of::<T>() <= BYTES && align_of::<T>() <= ALIGN
    }

    /// Like [`ArenaRef::place`], for the rest of the program: the value is
    /// never dropped and the arena can't be reused. For platforms whose
    /// `run` never returns.
    pub fn place_forever<T: Default>(self) -> &'static mut T {
        self.place_forever_with(Init::DEFAULT)
    }

    /// Like [`place_forever`](Self::place_forever), built the way `init`
    /// says.
    pub fn place_forever_with<T: Default>(self, init: Init) -> &'static mut T {
        let mut this = self;
        let placed = this.place_with::<T>(init);
        let ptr = placed.ptr;
        core::mem::forget(placed);
        // SAFETY: the arena is `'static` and this `ArenaRef`, its only
        // handle, is consumed here, so the value is never moved, dropped or
        // aliased.
        unsafe { &mut *ptr }
    }

    /// Build a `T` in place with `build`, for the rest of the program, if
    /// it fits; otherwise hand the arena back, untouched. The size check is
    /// a constant, so it folds away. For a platform that picks between
    /// arenas, and builds a struct field by field so that no part of it is
    /// ever on the stack.
    ///
    /// # Errors
    ///
    /// The arena, untouched, if `T` doesn't fit it.
    ///
    /// # Safety
    ///
    /// `build` must initialize every field of the `T` it is given a pointer
    /// to (aligned, valid for writes, uninitialized).
    pub unsafe fn try_place_forever_by<T>(
        self,
        build: impl FnOnce(*mut T),
    ) -> Result<&'static mut T, Self> {
        if !Self::fits::<T>() {
            return Err(self);
        }
        let mut this = self;
        // SAFETY: forwarded from the caller.
        let placed = unsafe { this.build_unchecked(build) };
        let ptr = placed.ptr;
        core::mem::forget(placed);
        // SAFETY: as in `place_forever`.
        Ok(unsafe { &mut *ptr })
    }

    /// Build a `T` in the arena with `build`.
    ///
    /// # Safety
    ///
    /// `T` fits, and `build` initializes all of it.
    unsafe fn build_unchecked<T>(&mut self, build: impl FnOnce(*mut T)) -> Placed<'_, T> {
        debug_assert!(Self::fits::<T>());
        let ptr = self.mem.cast::<T>();
        // `ptr` is aligned (the caller checked `fits`), points to `BYTES >=
        // size_of::<T>()` bytes this `ArenaRef` has exclusive access to, and
        // nothing else is placed there while the guard borrows `self`.
        build(ptr);
        Placed {
            ptr,
            _borrow: PhantomData,
        }
    }
}

/// How to build a value in place: with `Default`, or by zeroing its memory.
///
/// Building with `Default` returns the value and copies it into place, and
/// the optimizer does not reliably build it there directly: a graph with a
/// 256 KB delay line can put a 256 KB temporary on the stack, which on the
/// Daisy overflows DTCM. A graph whose default is all zeros (zeroed buffers,
/// `0.0`, `0`, `false`) can be built by zeroing the arena instead, which
/// never touches the stack. A device opts in with
/// [`Device::GRAPH_INIT`](crate::Device::GRAPH_INIT).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Init {
    zeroed: bool,
}

impl Init {
    /// Build with `T::default()`.
    pub const DEFAULT: Self = Self { zeroed: false };

    /// Build by zeroing the memory.
    ///
    /// # Safety
    ///
    /// All-zero bytes must be a valid value of every `T` this is used to
    /// build, and behave exactly like `T::default()`. That holds when every
    /// field's default is zero: `0.0`, `0`, `false`, zeroed arrays, `None`
    /// of a non-pointer type, the first variant of a plain enum.
    /// `euterpe_offline::check::check_init` tests the behavior.
    #[must_use]
    pub const unsafe fn zeroed() -> Self {
        Self { zeroed: true }
    }

    #[must_use]
    pub const fn is_zeroed(self) -> bool {
        self.zeroed
    }

    /// Build a `T` at `dst`.
    ///
    /// # Safety
    ///
    /// `dst` is aligned and valid for writes of a `T`.
    #[inline(always)]
    pub unsafe fn build<T: Default>(self, dst: *mut T) {
        // SAFETY: `dst` is valid (the caller), and zeroes are a valid `T`
        // when `zeroed` is set (the contract of `Init::zeroed`).
        unsafe {
            if self.zeroed {
                dst.write_bytes(0, 1);
            } else {
                dst.write(T::default());
            }
        }
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

    #[test]
    fn try_place_hands_back_an_arena_too_small() {
        static SMALL: Arena<16> = Arena::new();
        static BIG: Arena<64> = Arena::new();
        // SAFETY: the arena hands `build` an aligned, writable slot.
        let build = |p: *mut Counted| unsafe { p.write(Counted([1, 2, 3, 4])) };
        let small = SMALL.take().unwrap();
        // SAFETY: `build` writes the whole value.
        let Err(_) = (unsafe { small.try_place_forever_by(build) }) else {
            panic!("32 bytes don't fit in 16");
        };
        // SAFETY: as above.
        let v = unsafe { BIG.take().unwrap().try_place_forever_by(build) };
        assert_eq!(v.ok().unwrap().0, [1, 2, 3, 4]);
    }

    #[test]
    fn zeroed_init_builds_without_default() {
        #[derive(Debug, PartialEq)]
        struct NotDefault([f32; 8], u32);
        impl Default for NotDefault {
            fn default() -> Self {
                Self([1.0; 8], 7)
            }
        }
        static A: Arena<64> = Arena::new();
        let mut arena = A.take().unwrap();
        // Fill the memory first, so zeroing is visible.
        drop(arena.place::<NotDefault>());
        // SAFETY: (for this test) zeroes are a valid `NotDefault`; the point
        // is that `default()` is not called.
        let v = arena.place_with::<NotDefault>(unsafe { Init::zeroed() });
        assert_eq!(*v, NotDefault([0.0; 8], 0));
    }
}
