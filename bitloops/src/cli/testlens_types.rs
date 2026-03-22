use clap::ValueEnum;

pub const DEFAULT_QUERY_VIEW: QueryViewArg = QueryViewArg::Full;

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum QueryViewArg {
    Full,
    Summary,
    Tests,
    Coverage,
}
