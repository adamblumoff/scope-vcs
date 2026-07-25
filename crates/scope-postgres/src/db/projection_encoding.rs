pub(super) const LIVE_PROJECTION_SOURCE: &str = "live";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProjectionAudience {
    Private,
    Public,
}

impl ProjectionAudience {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Public => "public",
        }
    }
}
