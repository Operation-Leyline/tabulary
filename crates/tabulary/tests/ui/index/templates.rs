#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByValue, name = "by_value", kind = global, partition(name = "value", template = "{value:hex}"), project = all))]
struct FieldCollision {
    #[tabulary(partition_key)] id: String,
    value: String,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", partition(name = "pk", template = "{id:hex}"),
    index(ident = ByValue, name = "by_value", kind = global, partition(name = "pk", template = "{value:hex}"), project = all))]
struct TemplateCollision {
    id: String,
    value: String,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByValue, name = "by_value", kind = global, partition(name = "gpk", template = "{value:hex}"), project = all))]
struct OptionalSource {
    #[tabulary(partition_key)] id: String,
    value: Option<String>,
}
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case", index(ident = ByValue, name = "by_value", kind = global, partition(name = "gpk", template = "{value:hex}"), project = all))]
struct HandleCollision {
    #[tabulary(partition_key)] id: String,
    value: String,
    by_value_partition: String,
}
fn main() {}
