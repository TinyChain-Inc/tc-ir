use std::str::FromStr;

use destream::de::{self, FromStream};
use pathlink::{Link, PathBuf};

use crate::{After, Cond, ForEach, Id, Map, OpDef, OpRef, Scalar, Subject, TCRef, While};
use tc_value::Value;

#[derive(Clone, Copy)]
enum Root {
    Scalar,
    Ref,
    Op,
    Call,
    Params,
    Args,
}

#[derive(Clone)]
enum Tag {
    Value,
    Cond,
    After,
    While,
    ForEach,
    Get,
    Put,
    Post,
    Delete,
    Op(crate::op::OpDefType),
    Subject(String),
}

enum Target {
    Root(Root),
    Args,
    Form,
    Binding,
    Fields(Tag),
    Id,
    Subject,
}

enum Item {
    Scalar(Scalar),
    Ref(TCRef),
    Op(OpDef),
    Call(OpRef),
    Params(Map<Scalar>),
    Args(Args),
    Form(Vec<(Id, Scalar)>),
    Binding(Id, Scalar),
    Fields(Vec<Item>),
    Id(Id),
    Subject(Subject),
}

enum Args {
    Seq(Vec<Scalar>),
    Map(Map<Scalar>),
}

impl Item {
    fn scalar(self) -> Scalar {
        if let Self::Scalar(v) = self {
            v
        } else {
            unreachable!("scalar child")
        }
    }

    fn id(self) -> Id {
        if let Self::Id(v) = self {
            v
        } else {
            unreachable!("Id child")
        }
    }

    fn subject(self) -> Subject {
        if let Self::Subject(v) = self {
            v
        } else {
            unreachable!("subject child")
        }
    }

    fn form(self) -> Vec<(Id, Scalar)> {
        if let Self::Form(v) = self {
            v
        } else {
            unreachable!("form child")
        }
    }
}

enum Sequence {
    Tuple(Vec<Scalar>),
    Args(Vec<Scalar>),
    Form(Vec<(Id, Scalar)>),
    Binding(Vec<Item>),
    Fields(Tag, Vec<Item>),
}

impl Sequence {
    fn len(&self) -> usize {
        match self {
            Self::Tuple(items) | Self::Args(items) => items.len(),
            Self::Form(items) => items.len(),
            Self::Binding(items) | Self::Fields(_, items) => items.len(),
        }
    }

    fn push<E: de::Error>(&mut self, item: Item) -> Result<(), E> {
        match self {
            Self::Tuple(items) | Self::Args(items) => {
                items.try_reserve(1).map_err(E::custom)?;
                items.push(item.scalar());
            }
            Self::Form(items) => {
                let Item::Binding(id, value) = item else {
                    unreachable!("form binding")
                };
                items.try_reserve(1).map_err(E::custom)?;
                items.push((id, value));
            }
            Self::Binding(items) | Self::Fields(_, items) => {
                items.try_reserve(1).map_err(E::custom)?;
                items.push(item);
            }
        }
        Ok(())
    }
}

// Keep completed native payloads inline in the explicit continuation arena.
// Boxing every tagged value would add an allocation solely for traversal.
#[allow(clippy::large_enum_variant)]
enum Body {
    Sequence(Sequence),
    Map {
        root: Root,
        key: Option<String>,
        values: Map<Scalar>,
        ordinary: bool,
    },
    Tagged {
        root: Root,
        tag: Tag,
        result: Option<Item>,
        drain: bool,
    },
}

impl Body {
    fn depth(&self) -> usize {
        match self {
            Self::Sequence(Sequence::Tuple(_)) => 1,
            Self::Map {
                root: Root::Scalar,
                ordinary: true,
                ..
            } => 1,
            Self::Tagged { root, tag, .. } => {
                let scalar = usize::from(matches!(root, Root::Scalar));
                match tag {
                    Tag::Value => 0,
                    Tag::Op(_) => scalar + 1,
                    Tag::Cond | Tag::After | Tag::While | Tag::ForEach => scalar + 1,
                    Tag::Get | Tag::Put | Tag::Post | Tag::Delete | Tag::Subject(_) => {
                        scalar + usize::from(matches!(root, Root::Scalar | Root::Ref)) + 1
                    }
                }
            }
            _ => 0,
        }
    }
}

struct Frame {
    container: de::Container,
    body: Body,
}

fn tag<E: de::Error>(root: Root, key: String) -> Result<Tag, E> {
    let path = PathBuf::from_str(&key).ok();
    if !matches!(root, Root::Call) {
        if let Some(op) = path.as_ref().and_then(crate::op::OpDefType::from_path) {
            if matches!(root, Root::Scalar | Root::Op) {
                return Ok(Tag::Op(op));
            }
        }
        if matches!(root, Root::Op) {
            return Err(E::custom("expected Op definition type"));
        }
        for (label, tag) in [
            (crate::TCREF_COND, Tag::Cond),
            (crate::TCREF_AFTER, Tag::After),
            (crate::TCREF_WHILE, Tag::While),
            (crate::TCREF_FOR_EACH, Tag::ForEach),
        ] {
            if path.as_ref() == Some(&PathBuf::from(label)) {
                return Ok(tag);
            }
        }
    }
    for (label, tag) in [
        (crate::OPREF_GET, Tag::Get),
        (crate::OPREF_PUT, Tag::Put),
        (crate::OPREF_POST, Tag::Post),
        (crate::OPREF_DELETE, Tag::Delete),
    ] {
        if path.as_ref() == Some(&PathBuf::from(label)) {
            return Ok(tag);
        }
    }
    Ok(Tag::Subject(key))
}

fn payload(tag: &Tag) -> Target {
    match tag {
        Tag::Subject(_) => Target::Args,
        Tag::Op(crate::op::OpDefType::Post) => Target::Form,
        tag => Target::Fields(tag.clone()),
    }
}

fn expected<E: de::Error>(sequence: &Sequence) -> Result<Target, E> {
    let len = sequence.len();
    Ok(match sequence {
        Sequence::Tuple(_) | Sequence::Args(_) => Target::Root(Root::Scalar),
        Sequence::Form(_) => Target::Binding,
        Sequence::Binding(_) => match len {
            0 => Target::Id,
            1 => Target::Root(Root::Scalar),
            _ => return Err(E::invalid_length(len + 1, 2)),
        },
        Sequence::Fields(tag, _) => match tag {
            Tag::Cond if len == 0 => Target::Root(Root::Ref),
            Tag::Cond | Tag::While | Tag::ForEach if len < 3 => Target::Root(Root::Scalar),
            Tag::After if len < 2 => Target::Root(Root::Scalar),
            Tag::Get | Tag::Delete if len < 2 => {
                if len == 0 {
                    Target::Subject
                } else {
                    Target::Root(Root::Scalar)
                }
            }
            Tag::Put if len < 3 => {
                if len == 0 {
                    Target::Subject
                } else {
                    Target::Root(Root::Scalar)
                }
            }
            Tag::Post if len < 2 => {
                if len == 0 {
                    Target::Subject
                } else {
                    Target::Root(Root::Params)
                }
            }
            Tag::Op(crate::op::OpDefType::Get | crate::op::OpDefType::Delete) if len < 2 => {
                if len == 0 {
                    Target::Id
                } else {
                    Target::Form
                }
            }
            Tag::Op(crate::op::OpDefType::Put) if len < 3 => {
                if len < 2 {
                    Target::Id
                } else {
                    Target::Form
                }
            }
            _ => return Err(E::custom("too many operation fields")),
        },
    })
}

fn fields<E: de::Error>(item: Item, count: usize) -> Result<std::vec::IntoIter<Item>, E> {
    let Item::Fields(items) = item else {
        unreachable!("field payload")
    };
    if items.len() != count {
        return Err(E::invalid_length(items.len(), count));
    }
    Ok(items.into_iter())
}

fn call_from_args<E: de::Error>(subject: Subject, args: Args) -> Result<OpRef, E> {
    match args {
        Args::Map(params) => Ok(OpRef::Post((subject, params))),
        Args::Seq(items) => {
            let len = items.len();
            let mut items = items.into_iter();
            match len {
                1 => Ok(OpRef::Get((subject, items.next().unwrap()))),
                2 => Ok(OpRef::Put((
                    subject,
                    items.next().unwrap(),
                    items.next().unwrap(),
                ))),
                _ => Err(E::custom("invalid OpRef params (expected 1 or 2 elements)")),
            }
        }
    }
}

fn finish_tag<E: de::Error>(root: Root, tag: &Tag, item: Item) -> Result<Item, E> {
    let value = match tag {
        Tag::Value => return Ok(item),
        Tag::Cond => {
            let mut fields = fields::<E>(item, 3)?;
            let Item::Ref(cond) = fields.next().unwrap() else {
                return Err(E::custom("invalid Cond condition: expected a reference"));
            };
            Item::Ref(TCRef::Cond(Box::new(Cond::new(
                cond,
                fields.next().unwrap().scalar(),
                fields.next().unwrap().scalar(),
            ))))
        }
        Tag::After => {
            let mut fields = fields::<E>(item, 2)?;
            Item::Ref(TCRef::After(Box::new(After::new(
                fields.next().unwrap().scalar(),
                fields.next().unwrap().scalar(),
            ))))
        }
        Tag::While => {
            let mut fields = fields::<E>(item, 3)?;
            Item::Ref(TCRef::While(Box::new(While::new(
                fields.next().unwrap().scalar(),
                fields.next().unwrap().scalar(),
                fields.next().unwrap().scalar(),
            ))))
        }
        Tag::ForEach => {
            let mut fields = fields::<E>(item, 3)?;
            let items = fields.next().unwrap().scalar();
            let op = fields.next().unwrap().scalar();
            let item_name = fields.next().unwrap().scalar();
            let name = match item_name {
                Scalar::Value(Value::String(value)) => value.parse().map_err(E::custom)?,
                _ => return Err(E::custom("invalid ForEach item_name (expected string)")),
            };
            Item::Ref(TCRef::ForEach(Box::new(ForEach::new(items, op, name))))
        }
        Tag::Get | Tag::Delete => {
            let mut fields = fields::<E>(item, 2)?;
            let subject = fields.next().unwrap().subject();
            let key = fields.next().unwrap().scalar();
            Item::Call(if matches!(tag, Tag::Get) {
                OpRef::Get((subject, key))
            } else {
                OpRef::Delete((subject, key))
            })
        }
        Tag::Put => {
            let mut fields = fields::<E>(item, 3)?;
            Item::Call(OpRef::Put((
                fields.next().unwrap().subject(),
                fields.next().unwrap().scalar(),
                fields.next().unwrap().scalar(),
            )))
        }
        Tag::Post => {
            let mut fields = fields::<E>(item, 2)?;
            let subject = fields.next().unwrap().subject();
            let Item::Params(params) = fields.next().unwrap() else {
                unreachable!()
            };
            Item::Call(OpRef::Post((subject, params)))
        }
        Tag::Op(kind) => {
            let op = match kind {
                crate::op::OpDefType::Post => OpDef::Post(item.form()),
                crate::op::OpDefType::Get | crate::op::OpDefType::Delete => {
                    let mut fields = fields::<E>(item, 2)?;
                    let key = fields.next().unwrap().id();
                    let form = fields.next().unwrap().form();
                    if matches!(kind, crate::op::OpDefType::Get) {
                        OpDef::Get((key, form))
                    } else {
                        OpDef::Delete((key, form))
                    }
                }
                crate::op::OpDefType::Put => {
                    let mut fields = fields::<E>(item, 3)?;
                    OpDef::Put((
                        fields.next().unwrap().id(),
                        fields.next().unwrap().id(),
                        fields.next().unwrap().form(),
                    ))
                }
            };
            op.validate().map_err(E::custom)?;
            Item::Op(op)
        }
        Tag::Subject(key) => {
            let Item::Args(args) = item else {
                unreachable!()
            };
            if matches!(&args,Args::Seq(items) if items.is_empty()) && !matches!(root, Root::Call) {
                if key.starts_with('$') {
                    Item::Ref(TCRef::Id(key.parse().map_err(E::custom)?))
                } else if matches!(root, Root::Scalar) {
                    Item::Scalar(Value::from(Link::from_str(key).map_err(E::custom)?).into())
                } else {
                    return Err(E::custom("invalid OpRef params (expected 1 or 2 elements)"));
                }
            } else {
                let subject = crate::scalar::subject_from_str(key).map_err(E::custom)?;
                Item::Call(call_from_args(subject, args)?)
            }
        }
    };
    Ok(match (root, value) {
        (Root::Scalar, Item::Scalar(value)) => Item::Scalar(value),
        (Root::Scalar, Item::Ref(value)) => Item::Scalar(value.into()),
        (Root::Scalar, Item::Op(value)) => Item::Scalar(value.into()),
        (Root::Scalar, Item::Call(value)) => Item::Scalar(TCRef::Op(value).into()),
        (Root::Ref, Item::Call(value)) => Item::Ref(TCRef::Op(value)),
        (Root::Ref, value @ Item::Ref(_))
        | (Root::Op, value @ Item::Op(_))
        | (Root::Call, value @ Item::Call(_)) => value,
        _ => return Err(E::custom("invalid IR map type")),
    })
}

fn finish<E: de::Error>(body: Body) -> Result<Item, E> {
    Ok(match body {
        Body::Sequence(sequence) => match sequence {
            Sequence::Tuple(items) => Item::Scalar(Scalar::Tuple(items)),
            Sequence::Args(items) => Item::Args(Args::Seq(items)),
            Sequence::Form(items) => Item::Form(items),
            Sequence::Binding(items) => {
                if items.len() != 2 {
                    return Err(E::invalid_length(items.len(), 2));
                }
                let mut items = items.into_iter();
                Item::Binding(items.next().unwrap().id(), items.next().unwrap().scalar())
            }
            Sequence::Fields(_, items) => Item::Fields(items),
        },
        Body::Map { root, values, .. } => match root {
            Root::Scalar => Item::Scalar(Scalar::Map(values)),
            Root::Params => Item::Params(values),
            Root::Args => Item::Args(Args::Map(values)),
            _ => return Err(E::custom("expected nonempty IR type map")),
        },
        Body::Tagged { result, .. } => result.ok_or_else(|| E::custom("missing IR map payload"))?,
    })
}

async fn decode<D: de::Decoder>(
    target: Target,
    decoder: &mut D,
    remaining: usize,
) -> Result<Item, D::Error> {
    let mut pending: Vec<Frame> = Vec::new();
    let mut wanted = Some(target);
    let mut ready = None;
    loop {
        let used = pending
            .iter()
            .map(|frame| frame.body.depth())
            .sum::<usize>();
        let available = remaining.saturating_sub(used);
        if let Some(target) = wanted.take() {
            match target {
                Target::Id => ready = Some(Item::Id(Id::from_stream((), decoder).await?)),
                Target::Subject => {
                    ready = Some(Item::Subject(Subject::from_stream((), decoder).await?))
                }
                target => {
                    let kind = decoder.peek_kind().await?;
                    let body = match target {
                        Target::Root(Root::Scalar) if kind == de::Kind::Leaf => {
                            if used > remaining {
                                return Err(de::Error::custom("IR structural nesting exceeds 64"));
                            }
                            ready = Some(Item::Scalar(
                                tc_value::decode_value(decoder, available).await?.into(),
                            ));
                            continue;
                        }
                        Target::Root(Root::Scalar) if kind == de::Kind::Seq => {
                            Body::Sequence(Sequence::Tuple(Vec::new()))
                        }
                        Target::Root(root) if kind == de::Kind::Map => Body::Map {
                            root,
                            key: None,
                            values: Map::new(),
                            ordinary: matches!(root, Root::Params),
                        },
                        Target::Args if kind == de::Kind::Seq => {
                            Body::Sequence(Sequence::Args(Vec::new()))
                        }
                        Target::Args if kind == de::Kind::Map => Body::Map {
                            root: Root::Args,
                            key: None,
                            values: Map::new(),
                            ordinary: true,
                        },
                        Target::Form if kind == de::Kind::Seq => {
                            Body::Sequence(Sequence::Form(Vec::new()))
                        }
                        Target::Binding if kind == de::Kind::Seq => {
                            Body::Sequence(Sequence::Binding(Vec::new()))
                        }
                        Target::Fields(tag) if kind == de::Kind::Seq => {
                            Body::Sequence(Sequence::Fields(tag, Vec::new()))
                        }
                        _ => return Err(de::Error::custom("invalid IR container shape")),
                    };
                    let container = decoder.open_container(kind, None).await?;
                    pending.try_reserve(1).map_err(de::Error::custom)?;
                    pending.push(Frame { container, body });
                }
            }
            continue;
        }
        if let Some(item) = ready.take() {
            let Some(frame) = pending.last_mut() else {
                return Ok(item);
            };
            let parent_used = used - frame.body.depth();
            let parent_available = remaining.saturating_sub(parent_used);
            match &mut frame.body {
                Body::Sequence(sequence) => sequence.push(item)?,
                Body::Map {
                    root, key, values, ..
                } => {
                    let key = key.take().expect("map key");
                    let id = key.parse::<Id>().map_err(de::Error::custom)?;
                    if values.insert(id.clone(), item.scalar()).is_some()
                        && !matches!(root, Root::Args)
                    {
                        return Err(de::Error::custom(format!("duplicate map key {id}")));
                    }
                }
                Body::Tagged {
                    root, tag, result, ..
                } => {
                    let item = if matches!(tag, Tag::Subject(_)) {
                        match item {
                            Item::Params(values) => Item::Args(Args::Map(values)),
                            item => item,
                        }
                    } else {
                        item
                    };
                    let depth = match (&tag, &item) {
                        (Tag::Subject(key), Item::Args(Args::Seq(items)))
                            if items.is_empty() && !matches!(root, Root::Call) =>
                        {
                            usize::from(key.starts_with('$') && matches!(root, Root::Scalar))
                        }
                        _ => {
                            usize::from(matches!(root, Root::Scalar))
                                + 1
                                + usize::from(
                                    matches!(
                                        tag,
                                        Tag::Get
                                            | Tag::Put
                                            | Tag::Post
                                            | Tag::Delete
                                            | Tag::Subject(_)
                                    ) && matches!(root, Root::Scalar | Root::Ref),
                                )
                        }
                    };
                    if parent_used > remaining || depth > parent_available {
                        return Err(de::Error::custom("IR structural nesting exceeds 64"));
                    }
                    *result = Some(finish_tag(*root, tag, item)?);
                }
            }
            decoder.finish_child(&mut frame.container).await?;
            continue;
        }
        let frame = pending.last_mut().expect("pending IR container");
        match frame.container.slot() {
            de::Slot::End => {
                if (matches!(
                    &frame.body,
                    Body::Map {
                        root: Root::Scalar,
                        ordinary: false,
                        ..
                    }
                ) && available == 0)
                    || (used > remaining
                        && matches!(
                            &frame.body,
                            Body::Sequence(Sequence::Tuple(_))
                                | Body::Map {
                                    ordinary: true,
                                    root: Root::Scalar,
                                    ..
                                }
                        ))
                {
                    return Err(de::Error::custom("IR structural nesting exceeds 64"));
                }
                ready = Some(finish(pending.pop().unwrap().body)?);
            }
            de::Slot::Element => {
                let Body::Sequence(sequence) = &frame.body else {
                    unreachable!()
                };
                wanted = Some(expected(sequence)?);
            }
            de::Slot::Key => match &mut frame.body {
                Body::Map { key, .. } => {
                    *key = Some(String::from_stream((), decoder).await?);
                    decoder.finish_child(&mut frame.container).await?;
                }
                Body::Tagged { drain, .. } => {
                    if !*drain {
                        return Err(de::Error::custom("unexpected extra reference map entry"));
                    }
                    de::IgnoredAny::from_stream((), decoder).await?;
                    decoder.finish_child(&mut frame.container).await?;
                }
                _ => unreachable!(),
            },
            de::Slot::Value => match &mut frame.body {
                Body::Map {
                    root,
                    key,
                    ordinary,
                    ..
                } => {
                    let key = key.as_ref().expect("map key");
                    if *ordinary
                        || (matches!(root, Root::Scalar)
                            && !key.starts_with('/')
                            && !key.starts_with('$'))
                    {
                        *ordinary = true;
                        wanted = Some(Target::Root(Root::Scalar));
                        continue;
                    }
                    if matches!(root, Root::Scalar) && key.starts_with('/') {
                        if let Some(value) = tc_value::decode_typed_value(
                            key,
                            decoder,
                            &mut frame.container,
                            available,
                        )
                        .await?
                        {
                            frame.body = Body::Tagged {
                                root: *root,
                                tag: Tag::Value,
                                result: Some(Item::Scalar(value.into())),
                                drain: true,
                            };
                            continue;
                        }
                    }
                    let drain = matches!(root, Root::Call) || !key.starts_with('$');
                    let tag = tag(*root, key.clone())?;
                    wanted = Some(payload(&tag));
                    frame.body = Body::Tagged {
                        root: *root,
                        tag,
                        result: None,
                        drain,
                    };
                }
                Body::Tagged { result, .. } => {
                    debug_assert!(result.is_some());
                    de::IgnoredAny::from_stream((), decoder).await?;
                    decoder.finish_child(&mut frame.container).await?;
                }
                _ => unreachable!(),
            },
        }
    }
}

macro_rules! decode_root {
    ($ty:ty,$root:ident,$variant:ident) => {
        impl FromStream for $ty {
            type Context = ();
            async fn from_stream<D: de::Decoder>(
                (): (),
                decoder: &mut D,
            ) -> Result<Self, D::Error> {
                match decode(Target::Root(Root::$root), decoder, crate::MAX_SCALAR_DEPTH).await? {
                    Item::$variant(value) => Ok(value),
                    _ => unreachable!("IR root"),
                }
            }
        }
    };
}

decode_root!(Scalar, Scalar, Scalar);
decode_root!(TCRef, Ref, Ref);
decode_root!(OpDef, Op, Op);
decode_root!(OpRef, Call, Call);

/// Decode a Scalar within the remaining structural depth of its owner.
pub async fn decode_scalar<D: de::Decoder>(
    decoder: &mut D,
    remaining: usize,
) -> Result<Scalar, D::Error> {
    match decode(
        Target::Root(Root::Scalar),
        decoder,
        remaining.min(crate::MAX_SCALAR_DEPTH),
    )
    .await?
    {
        Item::Scalar(value) => Ok(value),
        _ => unreachable!("Scalar decoder root"),
    }
}

/// Decode a tagged operation payload using its enclosing owner's depth allowance.
/// Unknown paths leave the decoder untouched.
pub async fn decode_op_def_entry<D: de::Decoder>(
    path: &PathBuf,
    decoder: &mut D,
    map: &mut de::Container,
    remaining: usize,
) -> Result<Option<OpDef>, D::Error> {
    let Some(kind) = crate::op::OpDefType::from_path(path) else {
        return Ok(None);
    };
    let remaining = remaining
        .min(crate::MAX_SCALAR_DEPTH)
        .checked_sub(1)
        .ok_or_else(|| de::Error::custom("IR structural nesting exceeds 64"))?;
    let tag = Tag::Op(kind);
    let item = decode(payload(&tag), decoder, remaining).await?;
    let Item::Op(op) = finish_tag::<D::Error>(Root::Op, &tag, item)? else {
        unreachable!("operation payload")
    };
    decoder.finish_child(map).await?;
    Ok(Some(op))
}
