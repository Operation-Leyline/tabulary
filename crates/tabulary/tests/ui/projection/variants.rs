#[derive(tabulary::Variants)]
#[tabulary(tag = "kind", rename_all = "snake_case")]
enum State { Waiting, Sent { tracking: String } }
#[derive(tabulary::Record)]
#[tabulary(rename_all = "snake_case")]
struct Order { #[tabulary(partition_key)] id: String, #[tabulary(flatten)] state: State }
#[derive(tabulary::Projection)]
#[tabulary(record = Order)]
struct TagOnly { #[tabulary(attribute = OrderAttrs::STATE)] tag: StateTag }
#[derive(tabulary::Projection)]
#[tabulary(record = Order)]
struct PayloadOnly { #[tabulary(attribute = OrderAttrs::TRACKING)] tracking: String }
fn main() {}
