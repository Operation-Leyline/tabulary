use crate::{
    AttributePath, BuildError, BuiltCondition, BuiltUpdate, PathSegment, RenderedExpressions,
    aliases::AliasAllocator,
    condition::Operand,
    condition_renderer::render_condition,
    update::{ActionNode, SetNode},
};

fn target(node: &ActionNode) -> AttributePath {
    match node {
        ActionNode::NestedSet { target, .. } | ActionNode::NestedRemove { target } => {
            target.clone()
        }
        ActionNode::Set { target, .. }
        | ActionNode::Remove { target }
        | ActionNode::Add { target, .. }
        | ActionNode::Delete { target, .. } => {
            vec![PathSegment::Attribute(target.name.into())].into()
        }
    }
}
pub(crate) fn validate_targets(actions: &[ActionNode]) -> Result<(), BuildError> {
    let paths: Vec<_> = actions.iter().map(target).collect();
    for (i, first) in paths.iter().enumerate() {
        for second in &paths[i + 1..] {
            if first.segments().starts_with(second.segments())
                || second.segments().starts_with(first.segments())
            {
                return Err(BuildError::OverlappingUpdate {
                    first: first.clone(),
                    second: second.clone(),
                });
            }
        }
    }
    Ok(())
}
fn path(path: &AttributePath, aliases: &mut AliasAllocator) -> String {
    let mut text = String::new();
    for segment in path.segments() {
        match segment {
            PathSegment::Attribute(name) => {
                if !text.is_empty() {
                    text.push('.');
                }
                text.push_str(&aliases.name(name));
            }
            PathSegment::Index(index) => text.push_str(&format!("[{index}]")),
            PathSegment::Component(_) => unreachable!("update paths have no key components"),
        }
    }
    text
}
fn operand(value: &Operand, aliases: &mut AliasAllocator) -> String {
    match value {
        Operand::Attribute(field) => aliases.name(field.name),
        Operand::Size(field) => format!("size({})", aliases.name(field.name)),
        Operand::Value(value) => aliases.value(value),
    }
}
fn set(node: &SetNode, aliases: &mut AliasAllocator) -> String {
    match node {
        SetNode::Operand(value) => operand(value, aliases),
        SetNode::Path(value) => path(value, aliases),
        SetNode::IfNotExists { path, default } => {
            let name = aliases.name(path.name);
            format!("if_not_exists({name}, {})", operand(default, aliases))
        }
        SetNode::IfPathNotExists {
            path: value,
            default,
        } => {
            let name = path(value, aliases);
            format!("if_not_exists({name}, {})", operand(default, aliases))
        }
        SetNode::Plus(left, right)
        | SetNode::Minus(left, right)
        | SetNode::ListAppend(left, right) => {
            let left = set(left, aliases);
            let right = operand(right, aliases);
            match node {
                SetNode::Plus(..) => format!("{left} + {right}"),
                SetNode::Minus(..) => format!("{left} - {right}"),
                _ => format!("list_append({left}, {right})"),
            }
        }
    }
}
pub(crate) fn render_update(actions: &[ActionNode], aliases: &mut AliasAllocator) -> String {
    let mut groups: [Vec<&ActionNode>; 4] = Default::default();
    for action in actions {
        groups[match action {
            ActionNode::Set { .. } | ActionNode::NestedSet { .. } => 0,
            ActionNode::Remove { .. } | ActionNode::NestedRemove { .. } => 1,
            ActionNode::Add { .. } => 2,
            ActionNode::Delete { .. } => 3,
        }]
        .push(action);
    }
    groups
        .into_iter()
        .zip(["SET", "REMOVE", "ADD", "DELETE"])
        .filter(|(group, _)| !group.is_empty())
        .map(|(group, keyword)| {
            let actions: Vec<_> = group
                .into_iter()
                .map(|action| {
                    let target = path(&target(action), aliases);
                    match action {
                        ActionNode::Set { value, .. } | ActionNode::NestedSet { value, .. } => {
                            format!("{target} = {}", set(value, aliases))
                        }
                        ActionNode::Remove { .. } | ActionNode::NestedRemove { .. } => target,
                        ActionNode::Add { value, .. } | ActionNode::Delete { value, .. } => {
                            format!("{target} {}", operand(value, aliases))
                        }
                    }
                })
                .collect();
            format!("{keyword} {}", actions.join(", "))
        })
        .collect::<Vec<_>>()
        .join(" ")
}
impl<R> BuiltUpdate<R> {
    pub fn render(&self) -> RenderedExpressions {
        let mut result = RenderedExpressions::default();
        result.update = Some(render_update(self.actions(), &mut result.aliases));
        if let Some(condition) = self.condition() {
            result.condition = Some(render_condition(condition.root(), &mut result.aliases));
        }
        result
    }
    /// Allocate the update first, then its condition, in a single request space.
    pub fn render_with_condition(&self, condition: &BuiltCondition<R>) -> RenderedExpressions {
        let mut result = self.render();
        let condition = render_condition(condition.root(), &mut result.aliases);
        result.condition = Some(match result.condition {
            Some(guard) => format!("({guard} AND {condition})"),
            None => condition,
        });
        result
    }
}
