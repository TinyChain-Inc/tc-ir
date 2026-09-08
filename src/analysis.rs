use std::collections::{BTreeMap, BTreeSet};

use tc_error::{TCError, TCResult};

use crate::{ApplicationIdentity, Id, Method, OpDef, OpRef, Scalar, ScalarNode, Subject, TCRef};

/// Visit every leaf of a recursive literal member map exactly once.
pub fn visit_members<E>(
    members: crate::Map<Scalar>,
    mut visitor: impl FnMut(Box<[Id]>, Scalar) -> Result<(), E>,
) -> Result<(), E> {
    fn visit<E>(
        prefix: &mut Vec<Id>,
        members: crate::Map<Scalar>,
        visitor: &mut impl FnMut(Box<[Id]>, Scalar) -> Result<(), E>,
    ) -> Result<(), E> {
        for (name, member) in members {
            prefix.push(name);
            match member {
                Scalar::Map(children) => visit(prefix, children, visitor)?,
                member => visitor(prefix.clone().into(), member)?,
            }
            prefix.pop();
        }
        Ok(())
    }

    visit(&mut Vec::new(), members, &mut visitor)
}

/// An unresolved application dependency discovered in a canonical definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationRequirement {
    authority: Option<pathlink::Host>,
    methods: BTreeSet<Method>,
}

impl ApplicationRequirement {
    pub fn new(
        authority: Option<pathlink::Host>,
        methods: impl IntoIterator<Item = Method>,
    ) -> Self {
        Self {
            authority,
            methods: methods.into_iter().collect(),
        }
    }

    pub fn authority(&self) -> Option<&pathlink::Host> {
        self.authority.as_ref()
    }

    pub fn methods(&self) -> &BTreeSet<Method> {
        &self.methods
    }

    pub fn merge(&mut self, other: Self) -> Result<(), crate::ApplicationError> {
        match (&self.authority, other.authority) {
            (Some(left), Some(right)) if left != &right => {
                return Err(crate::ApplicationError::InvalidIdentity(
                    "one application dependency has conflicting authorities".to_string(),
                ));
            }
            (None, authority) => self.authority = authority,
            _ => {}
        }
        self.methods.extend(other.methods);
        Ok(())
    }
}

/// A deterministic dependency plan for one operation graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpPlan {
    levels: Vec<Vec<Id>>,
    dependencies: BTreeMap<Id, BTreeSet<Id>>,
}

impl OpPlan {
    pub fn compile<'a>(
        providers: impl IntoIterator<Item = (&'a Id, &'a Scalar)>,
        inputs: impl IntoIterator<Item = &'a Id>,
    ) -> TCResult<Self> {
        let providers = providers
            .into_iter()
            .map(|(id, scalar)| (id.clone(), scalar))
            .collect::<BTreeMap<_, _>>();
        let dependencies = providers
            .iter()
            .map(|(id, scalar)| (id.clone(), scalar.free_ids()))
            .collect::<BTreeMap<_, _>>();
        let inputs = inputs.into_iter().cloned().collect::<BTreeSet<_>>();
        if let Some(missing) = dependencies.values().flatten().find(|dependency| {
            !providers.contains_key(*dependency) && !inputs.contains(*dependency)
        }) {
            return Err(TCError::not_found(format!(
                "missing provider or input for id {missing}"
            )));
        }
        let mut remaining = providers.keys().cloned().collect::<BTreeSet<_>>();
        let mut resolved = BTreeSet::new();
        let mut levels = Vec::new();

        while !remaining.is_empty() {
            let level = remaining
                .iter()
                .filter(|id| {
                    dependencies
                        .get(*id)
                        .expect("provider dependency analysis")
                        .iter()
                        .filter(|dependency| providers.contains_key(*dependency))
                        .all(|dependency| resolved.contains(dependency))
                })
                .cloned()
                .collect::<Vec<_>>();
            if level.is_empty() {
                return Err(TCError::bad_request(
                    "op definition has circular dependencies",
                ));
            }
            for id in &level {
                remaining.remove(id);
                resolved.insert(id.clone());
            }
            levels.push(level);
        }

        Ok(Self {
            levels,
            dependencies,
        })
    }

    pub fn levels(&self) -> &[Vec<Id>] {
        &self.levels
    }

    pub fn dependencies(&self, id: &Id) -> Option<&BTreeSet<Id>> {
        self.dependencies.get(id)
    }

    pub fn required(&self, capture: &Id, providers: &BTreeSet<Id>) -> BTreeSet<Id> {
        let mut required = BTreeSet::new();
        let mut pending = vec![capture.clone()];
        while let Some(id) = pending.pop() {
            if !required.insert(id.clone()) {
                continue;
            }
            pending.extend(
                self.dependencies
                    .get(&id)
                    .into_iter()
                    .flatten()
                    .filter(|dependency| providers.contains(*dependency))
                    .cloned(),
            );
        }
        required
    }
}

pub fn application_requirements<'a>(
    scalars: impl IntoIterator<Item = &'a Scalar>,
) -> Result<BTreeMap<ApplicationIdentity, ApplicationRequirement>, crate::ApplicationError> {
    let mut requirements: BTreeMap<ApplicationIdentity, ApplicationRequirement> = BTreeMap::new();
    let mut error = None;
    for scalar in scalars {
        scalar.visit(&mut |node| {
            let ScalarNode::Op(op) = node else {
                return true;
            };
            let (method, subject) = match op {
                OpRef::Get((subject, _)) => (Method::Get, subject),
                OpRef::Put((subject, _, _)) => (Method::Put, subject),
                OpRef::Post((subject, _)) => (Method::Post, subject),
                OpRef::Delete((subject, _)) => (Method::Delete, subject),
            };
            let Subject::Link(link) = subject else {
                return true;
            };
            let Some(root) = link.path().first() else {
                return true;
            };
            if !matches!(root.as_str(), "lib" | "class" | "service") {
                return true;
            }
            match crate::ApplicationTarget::split_segments(link.path()) {
                Ok((identity, _)) => {
                    let authority = link.host().cloned();
                    match requirements.entry(identity) {
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            entry.insert(ApplicationRequirement {
                                authority,
                                methods: BTreeSet::from([method]),
                            });
                        }
                        std::collections::btree_map::Entry::Occupied(mut entry) => {
                            if entry.get().authority != authority {
                                error = Some(crate::ApplicationError::InvalidIdentity(format!(
                                    "application {} is referenced through conflicting authorities",
                                    entry.key()
                                )));
                            } else {
                                entry.get_mut().methods.insert(method);
                            }
                        }
                    }
                }
                Err(cause) => error = Some(cause),
            }
            true
        });
    }
    error.map_or(Ok(requirements), Err)
}

impl Scalar {
    pub fn free_ids(&self) -> BTreeSet<Id> {
        let mut required = BTreeSet::new();
        self.visit(&mut |node| {
            match node {
                ScalarNode::Scalar(Scalar::Op(op)) => {
                    required.extend(op.free_ids());
                    return false;
                }
                ScalarNode::Ref(TCRef::Id(id)) if id.as_str() != "self" => {
                    required.insert(id.id().clone());
                }
                ScalarNode::Subject(Subject::Ref(id, _)) if id.as_str() != "self" => {
                    required.insert(id.id().clone());
                }
                _ => {}
            }
            true
        });
        required
    }

    pub fn application_requirements(
        &self,
    ) -> Result<BTreeMap<ApplicationIdentity, ApplicationRequirement>, crate::ApplicationError>
    {
        application_requirements(std::iter::once(self))
    }
}

impl OpDef {
    pub fn free_ids(&self) -> BTreeSet<Id> {
        let mut required = BTreeSet::new();
        for (_, scalar) in self.form() {
            required.extend(scalar.free_ids());
        }
        for (defined, _) in self.form() {
            required.remove(defined);
        }
        match self {
            OpDef::Get((key, _)) | OpDef::Delete((key, _)) => {
                required.remove(key);
            }
            OpDef::Put((key, value, _)) => {
                required.remove(key);
                required.remove(value);
            }
            OpDef::Post(_) => {}
        }
        required
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IdRef, Map};

    fn id(value: &str) -> Id {
        value.parse().expect("id")
    }

    #[test]
    fn plan_is_deterministic_and_rejects_cycles() {
        let a = Scalar::from(TCRef::Id(IdRef::from(id("input"))));
        let b = Scalar::from(TCRef::Id(IdRef::from(id("a"))));
        let providers = BTreeMap::from([(id("b"), b), (id("a"), a)]);
        let inputs = BTreeSet::from([id("input")]);
        let plan = OpPlan::compile(&providers, &inputs).expect("plan");
        assert_eq!(plan.levels(), &[vec![id("a")], vec![id("b")]]);

        let cycle = BTreeMap::from([
            (id("a"), Scalar::from(TCRef::Id(IdRef::from(id("b"))))),
            (id("b"), Scalar::from(TCRef::Id(IdRef::from(id("a"))))),
        ]);
        assert!(OpPlan::compile(&cycle, &BTreeSet::new()).is_err());

        let missing =
            BTreeMap::from([(id("a"), Scalar::from(TCRef::Id(IdRef::from(id("missing")))))]);
        assert!(OpPlan::compile(&missing, &BTreeSet::new()).is_err());
    }

    #[test]
    fn member_visitor_consumes_structure_once_and_reports_leaf_paths() {
        let mut nested = Map::new();
        nested.insert(id("answer"), Scalar::from(42_u64));
        let mut members = Map::new();
        members.insert(id("nested"), Scalar::Map(nested));
        members.insert(id("ready"), Scalar::Value(tc_value::Value::from(true)));
        let mut paths = Vec::new();
        visit_members(members, |path, _| {
            paths.push(path.iter().map(ToString::to_string).collect::<Vec<_>>());
            Ok::<_, std::convert::Infallible>(())
        })
        .expect("visit members");
        assert_eq!(paths, [vec!["nested", "answer"], vec!["ready"]]);
    }

    #[test]
    fn nested_op_providers_are_not_outer_free_ids() {
        let nested = OpDef::Post(vec![
            (id("local"), Scalar::from(1_u64)),
            (
                id("result"),
                Scalar::from(TCRef::Id(IdRef::from(id("local")))),
            ),
        ]);
        assert!(Scalar::Op(nested).free_ids().is_empty());
    }

    #[test]
    fn application_methods_are_aggregated() {
        let target: pathlink::Link = "/lib/example-devco/math/1.0.0/add".parse().expect("link");
        let scalar = Scalar::Tuple(vec![
            Scalar::from(TCRef::Op(OpRef::Get((
                Subject::Link(target.clone()),
                Scalar::default(),
            )))),
            Scalar::from(TCRef::Op(OpRef::Post((Subject::Link(target), Map::new())))),
        ]);
        let requirements = scalar.application_requirements().expect("requirements");
        let identity: ApplicationIdentity =
            "/lib/example-devco/math/1.0.0".parse().expect("identity");
        assert_eq!(
            requirements[&identity].methods(),
            &BTreeSet::from([Method::Get, Method::Post])
        );
    }

    #[test]
    fn application_authority_conflicts_are_rejected() {
        let local: pathlink::Link = "/lib/example-devco/math/1.0.0/add".parse().unwrap();
        let remote: pathlink::Link = "http://example.com/lib/example-devco/math/1.0.0/add"
            .parse()
            .unwrap();
        let scalar = Scalar::Tuple(vec![
            Scalar::from(TCRef::Op(OpRef::Get((
                Subject::Link(local),
                Scalar::default(),
            )))),
            Scalar::from(TCRef::Op(OpRef::Get((
                Subject::Link(remote),
                Scalar::default(),
            )))),
        ]);
        assert!(scalar.application_requirements().is_err());
    }
}
