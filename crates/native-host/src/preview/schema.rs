//! Map-only wire structs: preserve Serde's field and duplicate checks, reject positional arrays.
use serde::Deserialize;

pub(crate) fn deserialize_object<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where D: serde::Deserializer<'de>, T: Deserialize<'de> {
    struct Object<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Object<T> {
        type Value = T;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("a JSON object") }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<T, M::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    deserializer.deserialize_map(Object(std::marker::PhantomData))
}

macro_rules! object_struct {
    ($(#[$attribute:meta])* $visibility:vis struct $name:ident {
        $($(#[$field_attribute:meta])* $field_visibility:vis $field:ident: $type:ty),* $(,)?
    }) => {
        #[derive(Clone, Debug, serde::Serialize)]
        $(#[$attribute])*
        $visibility struct $name {
            $($(#[$field_attribute])* $field_visibility $field: $type),*
        }
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(serde::Deserialize)]
                $(#[$attribute])*
                struct Fields { $($(#[$field_attribute])* $field: $type),* }
                let fields: Fields = $crate::preview::schema::deserialize_object(deserializer)?;
                Ok(Self { $($field: fields.$field),* })
            }
        }
    };
}
pub(crate) use object_struct;
