use type_signature::TypeSignature;

use crate::TypeHash;

impl<T: TypeSignature> TypeHash for T {
    const HASH: u64 = T::CONST_HASH;
}
