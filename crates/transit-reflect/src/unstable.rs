#[path = "unstable/stable_std.rs"]
mod stable_std;

use std::{
    any::TypeId,
    mem::type_info::{self, TypeKind},
};

use xxhash_rust::const_xxh3;

use crate::{TypeHash, common::Hasher};

macro_rules! match_against {
    ($it:expr => $z:expr; $id:ty $(, $($rem:ty),+ $(,)?)?) => {
        if $it == type_info::of::<$id>() {
            return $z;
        }

        $(match_against!($it => $z + 1; $($rem),+);)?
    };
}

#[rustc_comptime]
fn hash_of(id: TypeId) -> u64 {
    const PRIMITIVE_START: u64 = 0;
    const INT_START: u64 = 10;
    const FLOAT_START: u64 = 30;

    const SLICE: u8 = 40;
    const REFLIKE_START: u8 = 41;

    const ENUM: u8 = 50;
    const STRUCT: u8 = 51;
    const UNION: u8 = 52;
    const TUPLE: u8 = 53;
    const FN_PTR: u8 = 54;

    const ENUM_VARIANT: u8 = 60;
    const FN_PTR_OUTPUT: u8 = 61;

    if let Some(h) = stable_std::try_hash(id) {
        return h;
    }

    let k = id.info().kind;
    match k {
        // primitive types
        TypeKind::Bool => PRIMITIVE_START,
        TypeKind::Char => PRIMITIVE_START + 1,
        TypeKind::Str(_) => PRIMITIVE_START + 2,
        TypeKind::Int => {
            match_against!(
                id => INT_START;
                u8, u16, u32, u64, u128, usize,
                i8, i16, i32, i64, i128, isize,
            );

            panic!("unknown type")
        }
        TypeKind::Float => {
            match_against!(
                id => FLOAT_START;
                f16, f32, f64, f128
            );

            panic!("unknown type")
        }

        // one-arg types
        TypeKind::Array => {
            let inner = id.element_ty().expect("Expected an array?");
            let len = (id.array_len() as u64).to_le_bytes();
            let inner_hash = hash_of(inner).to_le_bytes();

            let mut comb = [0u8; 16];
            comb[..8].copy_from_slice(&inner_hash);
            comb[8..].copy_from_slice(&len);

            const_xxh3::xxh3_64(&comb)
        }
        TypeKind::Slice => {
            let inner = id.element_ty().expect("Expected an array?");
            let inner_hash = hash_of(inner).to_le_bytes();

            let mut comb = [0u8; 9];
            comb[0] = SLICE;
            comb[1..].copy_from_slice(&inner_hash);

            const_xxh3::xxh3_64(&comb)
        }
        TypeKind::Pointer | TypeKind::Reference => {
            let inner = id.points_to().expect("Expected a pointer?");
            let inner_hash = hash_of(inner).to_le_bytes();
            let mutable = id.points_mutably();
            let is_ptr = matches!(k, TypeKind::Pointer);

            let mut comb = [0u8; 9];
            comb[0] = REFLIKE_START + (2 * is_ptr as u8) + mutable as u8;
            comb[1..].copy_from_slice(&inner_hash);

            const_xxh3::xxh3_64(&comb)
        }

        // variable types
        TypeKind::DynTrait(_) => panic!("I refuse to deal with dyn Traits"),
        TypeKind::Enum => {
            let variants = id.variants();
            let mut hb = Hasher::new(ENUM);

            let mut i = 0;
            while i < variants {
                let var = id.variant(i);
                let n_fields = id.fields(i);

                let mut hb2 = Hasher::new(ENUM_VARIANT);
                hb2.next(var.name().as_bytes());

                let mut j = 0;
                while j < n_fields {
                    let field = id.field(i, j);
                    hb2.next(field.name().as_bytes());
                    hb2.next(&hash_of(field.type_id()).to_le_bytes());

                    j += 1;
                }

                hb.combine(hb2);
                i += 1;
            }

            hb.finish()
        }
        TypeKind::Struct | TypeKind::Tuple | TypeKind::Union => {
            let fields = id.fields(0);
            let mut hb = Hasher::new(match k {
                TypeKind::Struct => STRUCT,
                TypeKind::Tuple => TUPLE,
                TypeKind::Union => UNION,
                _ => unreachable!(),
            });

            let mut i = 0;
            while i < fields {
                let field = id.field(0, i);
                hb.next(field.name().as_bytes());
                hb.next(&hash_of(field.type_id()).to_le_bytes());

                i += 1;
            }

            hb.finish()
        }
        TypeKind::FnPtr => {
            let info = id.function_ptr().expect("Expected a function pointer");
            let mut hb = Hasher::new(FN_PTR);

            let mut i = 0;
            while i < info.inputs().len() {
                hb.next(&hash_of(info.inputs()[i]).to_le_bytes());
                i += 1;
            }

            hb.next(&[FN_PTR_OUTPUT]);
            hb.next(&hash_of(info.output()).to_le_bytes());
            hb.finish()
        }
        _ => panic!("Unknown type"),
    }
}

impl<T> TypeHash for T {
    const HASH: u64 = hash_of(type_info::of::<T>());
}
