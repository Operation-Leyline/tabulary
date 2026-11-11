//! Type-level DynamoDB categories shared by native values and custom converters.

use crate::DynamoType;

/// The DynamoDB category produced and accepted by a codec.
pub trait WireType {
    const DYNAMO_TYPE: DynamoType;
}

macro_rules! markers {
    ($($name:ident => $tag:ident),* $(,)?) => {$ (
        #[doc = concat!("The `", stringify!($tag), "` wire category.")]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct $name;

        impl WireType for $name {
            const DYNAMO_TYPE: DynamoType = DynamoType::$tag;
        }
    )*};
}

markers! {
    Binary => B,
    BinarySet => Bs,
    Bool => Bool,
    List => L,
    Map => M,
    Null => Null,
    Number => N,
    NumberSet => Ns,
    String => S,
    StringSet => Ss,
}
