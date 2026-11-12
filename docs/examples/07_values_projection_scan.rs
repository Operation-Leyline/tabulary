use aws_sdk_dynamodb::Client;
use std::{collections::BTreeMap, error::Error, num::NonZeroU32};
use tabulary::{
    BinarySet, Decimal, Document, List, Map, Nullable, NumberSet, Projection, ReadConsistency,
    Record, ScanSegment, Secret, StringEnum, StringSet, Table, TimestampMillis, TtlSeconds, Update,
};

#[derive(Debug, PartialEq, StringEnum)]
#[tabulary(rename_all = "snake_case")]
pub enum SettlementUnit {
    Token,
    Voucher,
}

#[derive(Debug, PartialEq, Document)]
#[tabulary(rename_all = "snake_case")]
pub struct Postal {
    destination_city: String,
    destination_zip: String,
}

fn initial_quantity() -> u64 {
    1
}

#[derive(Record)]
#[tabulary(rename_all = "snake_case")]
pub struct InvoiceEnvelope {
    #[tabulary(partition_key)]
    envelope_ref: String,
    #[tabulary(default = initial_quantity)]
    quantity: u64,
    credit_adjustment: i64,
    external_number: u128,
    #[tabulary(with = tabulary::convert::u128_string)]
    full_width_number: u128,
    ratio: f64,
    exact_price: Decimal,
    attachment: Vec<u8>,
    signature_bytes: [u8; 16],
    verified: bool,
    labels: StringSet,
    denominations: NumberSet<u64>,
    signatures: BinarySet,
    notes: List<String>,
    postal: Postal,
    properties: Map<String>,
    optional_note: Option<String>,
    explicit_null: Nullable<String>,
    settlement_unit: SettlementUnit,
    issued_on: TimestampMillis,
    #[tabulary(ttl)]
    purge_on: Option<TtlSeconds>,
    private_note: Secret<String>,
}

#[derive(Projection)]
#[tabulary(record = InvoiceEnvelope)]
pub struct QuantityView {
    #[tabulary(attribute = InvoiceEnvelopeAttrs::QUANTITY)]
    quantity: u64,
}

pub fn envelope() -> Result<InvoiceEnvelope, Box<dyn Error>> {
    Ok(InvoiceEnvelope {
        envelope_ref: "envelope-saffron".into(),
        quantity: 2,
        credit_adjustment: -3,
        external_number: 123,
        full_width_number: u128::MAX,
        ratio: 0.5,
        exact_price: "19.75".parse::<Decimal>().map_err(|kind| {
            tabulary::DecodeContext::new("InvoiceEnvelope")
                .at(tabulary::PathSegment::Attribute("exact_price".into()))
                .error(kind)
        })?,
        attachment: vec![0x2a, 0x2b],
        signature_bytes: [0x6e; 16],
        verified: true,
        labels: StringSet::try_from(vec!["gift".to_owned(), "priority".to_owned()])?,
        denominations: NumberSet::try_from(vec![2_u64, 5])?,
        signatures: BinarySet::try_from(vec![vec![0x31], vec![0x32]])?,
        notes: List::from(vec!["wrap in paper".to_owned()]),
        postal: Postal {
            destination_city: "Exampleton".into(),
            destination_zip: "EX-12".into(),
        },
        properties: Map::from(BTreeMap::from([("paper".to_owned(), "blue".to_owned())])),
        optional_note: None,
        explicit_null: Nullable::Null,
        settlement_unit: SettlementUnit::Voucher,
        issued_on: TimestampMillis::new(12_000),
        purge_on: Some(TtlSeconds::new(86_400)),
        private_note: Secret::new("keep inside the package".into()),
    })
}

pub async fn projected_segment(client: &Client) -> Result<Vec<QuantityView>, Box<dyn Error>> {
    let envelopes = Table::<InvoiceEnvelope>::new("invoice_envelopes")?;
    let request = envelopes
        .scan(ReadConsistency::Strong)
        .segment(ScanSegment::new(1, 3)?)
        .select::<QuantityView>()
        .limit(NonZeroU32::new(5).unwrap())
        .build()?;
    let raw = request.apply(client.scan()).send().await;
    let page = request.decode(raw)?;
    // A caller scanning the entire segment retains page.next separately.
    Ok(page.items)
}

pub fn nested_changes() -> Update<InvoiceEnvelope> {
    Update::new()
        .and(
            InvoiceEnvelopeAttrs::POSTAL
                .field(PostalAttrs::DESTINATION_CITY)
                .set("Otherburg".into()),
        )
        .and(InvoiceEnvelopeAttrs::OPTIONAL_NOTE.remove())
        .and(InvoiceEnvelopeAttrs::NOTES.list_append(List::from(vec!["include a card".to_owned()])))
}
