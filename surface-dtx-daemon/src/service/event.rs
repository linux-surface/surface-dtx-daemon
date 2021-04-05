use crate::logic::CancelReason;
use crate::service::arg::DbusArg;

use dbus::arg::{Append, Dict, RefArg, Variant};



pub enum Event {
    DetachmentStart,
    DetachmentComplete,
    DetachmentTimeout,
    DetachmentCancelStart { reason: CancelReason },
    DetachmentCancelComplete,
    DetachmentCancelTimeout,
}

impl dbus::arg::AppendAll for Event {
    fn append(&self, ia: &mut dbus::arg::IterAppend) {
        match self {
            Event::DetachmentStart                  => append0(ia, "detachment:start"),
            Event::DetachmentComplete               => append0(ia, "detachment:complete"),
            Event::DetachmentTimeout                => append0(ia, "detachment:timeout"),
            Event::DetachmentCancelStart { reason } => append1(ia, "detachment:cancel:start", "reason", reason),
            Event::DetachmentCancelComplete         => append0(ia, "detachment:cancel:complete"),
            Event::DetachmentCancelTimeout          => append0(ia, "detachment:cancel:timeout"),
        }
    }
}

fn append0(ia: &mut dbus::arg::IterAppend, ty: &'static str) {
    let values: Dict<String, Variant<Box<dyn RefArg>>, _> = Dict::new(std::iter::empty());

    ty.append(ia);
    values.append(ia);
}

fn append1<T>(ia: &mut dbus::arg::IterAppend, ty: &'static str, name: &'static str, value: &T)
where
    T: DbusArg,
{
    ty.append(ia);

    ia.append_dict(&"s".into(), &"v".into(), |ia| {
        ia.append_dict_entry(|ia| {
            ia.append(name.to_owned());
            ia.append(value.as_variant());
        })
    });
}
