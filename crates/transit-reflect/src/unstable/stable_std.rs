// until we get *really* nice typeinfo (which seems to be a priority for rustc,
// nice.) we have to do this shit in case rust changes the internal
// representation of things, fucking us up

use std::{
    any::TypeId,
    collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet, LinkedList, VecDeque},
    marker::PhantomData,
    mem::type_info::Generic,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    num::{
        NonZeroI8, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI128, NonZeroIsize, NonZeroU8,
        NonZeroU16, NonZeroU32, NonZeroU64, NonZeroU128, NonZeroUsize,
    },
    rc::Rc,
    sync::{
        Arc,
        atomic::{
            AtomicBool, AtomicI8, AtomicI16, AtomicI32, AtomicI64, AtomicIsize, AtomicU8,
            AtomicU16, AtomicU32, AtomicU64, AtomicUsize,
        },
    },
    time::Duration,
};

use crate::common::Hasher;

trait Family<Tag> {}

macro_rules! unit {
    ($x:ident) => {
        ()
    };
}

const STABLE_STD: u8 = 70;

macro_rules! stable_std {
    (
        plain: $($p:ident),* $(,)? ;
        generic: $($a:ident < $($inner:ident $(: ?$q:ident)?),* >),* $(,)?
    ) => {
        $( impl Family<$p> for $p {} )*
        $(
            impl<$($inner $(: ?$q)?),*> Family<$a<$(unit!($inner)),*>> for $a<$($inner),*> {}
        )*

        #[rustc_comptime]
        pub(super) fn try_hash(id: TypeId) -> Option<u64> {
            $(
                if id.trait_info_of::<dyn Family<$p>>().is_some() {
                    return Some(with_generics(id, stringify!($p), 0));
                }
            )*
            $(
                if id.trait_info_of::<dyn Family<$a<$(unit!($inner)),*>>>().is_some() {
                    let n = <[()]>::len(&[$(unit!($inner)),*]);
                    return Some(with_generics(id, stringify!($a), n));
                }
            )*
            None
        }
    };
}

#[rustc_comptime]
fn with_generics(id: TypeId, name: &str, n: usize) -> u64 {
    let mut hb = Hasher::new(STABLE_STD);
    hb.next(name.as_bytes());

    let generics = id.generics();
    let (mut i, mut seen) = (0, 0);
    while i < generics.len() && seen < n {
        if let Generic::Type(t) = &generics[i] {
            hb.next(&super::hash_of(t.ty).to_le_bytes());
            seen += 1;
        }

        i += 1;
    }

    hb.finish()
}

#[rustfmt::skip]
stable_std!(
    plain:
        // NonZero types
        NonZeroU8, NonZeroU16, NonZeroU32, NonZeroU64, NonZeroU128,
        NonZeroI8, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI128,
        NonZeroUsize, NonZeroIsize,

        // Atomic types
        AtomicBool,
        AtomicU8, AtomicU16, AtomicU32, AtomicU64, AtomicUsize,
        AtomicI8, AtomicI16, AtomicI32, AtomicI64, AtomicIsize,

        // IP Addresses
        Ipv4Addr, Ipv6Addr, IpAddr,
        SocketAddrV4, SocketAddrV6, SocketAddr,

        // Other stuff
        String, Duration;

    generic:
        // Making rust great
        Option<T>, Result<T, E>,

        // Arraylike
        Vec<T>, VecDeque<T>, LinkedList<T>,

        // Hash
        HashSet<T>, HashMap<K, V>,

        // Stuff you only use in your algorithms class
        BTreeSet<T>, BinaryHeap<T>, BTreeMap<K, V>,

        // Pointerlike
        Box<T: ?Sized>, Rc<T: ?Sized>, Arc<T: ?Sized>,

        // Making rust a pain
        PhantomData<T: ?Sized>,
);
