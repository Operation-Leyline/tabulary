use tabulary::{MultiObject, Record};

#[derive(Record)]
#[tabulary(rename_all = "snake_case",
    partition(name = "pk", template = "ORD#{order_ref:text}"),
    sort(name = "sk", template = "LINE#{line_no:dec20}"))]
struct OrderLine {
    order_ref: String,
    line_no: u64,
}

#[derive(Record)]
#[tabulary(rename_all = "snake_case",
    partition(name = "pk", template = "ORD#{order_ref:text}"),
    sort(name = "sk", template = "LINE#X{seq:dec20}"))]
struct OrderNote {
    order_ref: String,
    seq: u64,
}

// `LINE#` is a prefix of `LINE#X`, and both share the partition prefix.
#[derive(MultiObject)]
#[tabulary(partition = "pk", sort = "sk")]
enum OrderTable {
    #[tabulary(partition(prefix = "ORD#"), sort(prefix = "LINE#"))]
    Line(OrderLine),
    #[tabulary(partition(prefix = "ORD#"), sort(prefix = "LINE#X"))]
    Note(OrderNote),
}

// Partition prefixes overlap and no sort key separates the members.
#[derive(MultiObject)]
#[tabulary(partition = "pk", sort = "sk")]
enum SharedPartition {
    #[tabulary(partition(prefix = "ORD#"))]
    Line(OrderLine),
    #[tabulary(partition(prefix = "ORD"), sort(prefix = "LINE#X"))]
    Note(OrderNote),
}

// An exact literal that begins with another member's prefix is ambiguous.
#[derive(MultiObject)]
#[tabulary(partition = "pk", sort = "sk")]
enum ExactInsidePrefix {
    #[tabulary(partition(prefix = "ORD#"), sort(prefix = "LINE#"))]
    Line(OrderLine),
    #[tabulary(partition(prefix = "ORD#"), sort(exact = "LINE#X"))]
    Note(OrderNote),
}

fn main() {}
