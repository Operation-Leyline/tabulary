use crate::{
    AttributePath, BuildError, BuiltCondition, BuiltUpdate, PathSegment, RenderedExpressions,
    aliases::AliasAllocator,
    condition_renderer::render_condition,
    update::{ActionNode, SetNode},
};

fn target(node: &ActionNode) -> AttributePath {
    match node {
        ActionNode::NestedSet { target, .. }
        | ActionNode::NestedRemove { target }
        | ActionNode::NestedAdd { target, .. } => target.clone(),
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
fn set(node: &SetNode, aliases: &mut AliasAllocator) -> String {
    match node {
        SetNode::Operand(value) => aliases.operand(value),
        SetNode::Path(value) => aliases.path(value),
        SetNode::IfNotExists { path, default } => {
            let name = aliases.name(path.name);
            format!("if_not_exists({name}, {})", aliases.operand(default))
        }
        SetNode::IfPathNotExists { path, default } => {
            let name = aliases.path(path);
            format!("if_not_exists({name}, {})", aliases.operand(default))
        }
        SetNode::Plus(left, right)
        | SetNode::Minus(left, right)
        | SetNode::ListAppend(left, right) => {
            let left = set(left, aliases);
            let right = aliases.operand(right);
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
            ActionNode::Add { .. } | ActionNode::NestedAdd { .. } => 2,
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
                    let target = aliases.path(&target(action));
                    match action {
                        ActionNode::Set { value, .. } | ActionNode::NestedSet { value, .. } => {
                            format!("{target} = {}", set(value, aliases))
                        }
                        ActionNode::Remove { .. } | ActionNode::NestedRemove { .. } => target,
                        ActionNode::Add { value, .. }
                        | ActionNode::NestedAdd { value, .. }
                        | ActionNode::Delete { value, .. } => {
                            format!("{target} {}", aliases.operand(value))
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
