//! String unions that cross the IPC boundary.
//!
//! The Rust structs keep plain `String` fields (their code compares and builds
//! strings), and each field carries `#[specta(type = ...)]` pointing at one of
//! these enums so the generated TypeScript gets the exact union. The variants
//! only describe the wire values; they are never constructed in Rust.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum AiKeySource {
    #[serde(rename = "windowsCredentialManager")]
    WindowsCredentialManager,
    #[serde(rename = "environment")]
    Environment,
    #[serde(rename = "none")]
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum AiQueryTarget {
    #[serde(rename = "search")]
    Search,
    #[serde(rename = "chart")]
    Chart,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum BrowseView {
    #[serde(rename = "albums")]
    Albums,
    #[serde(rename = "tracks")]
    Tracks,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum AiCompiledQueryQueryIntent {
    #[serde(rename = "filter")]
    Filter,
    #[serde(rename = "answer")]
    Answer,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum AiMusicResearchEntity {
    #[serde(rename = "album")]
    Album,
    #[serde(rename = "artist")]
    Artist,
    #[serde(rename = "genre")]
    Genre,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum AiMusicResearchTurnRole {
    #[serde(rename = "user")]
    User,
    #[serde(rename = "assistant")]
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum AiLibraryLens {
    #[serde(rename = "overview")]
    Overview,
    #[serde(rename = "ratingBacklog")]
    RatingBacklog,
    #[serde(rename = "tasteProfile")]
    TasteProfile,
    #[serde(rename = "catalogBalance")]
    CatalogBalance,
    #[serde(rename = "metadataHealth")]
    MetadataHealth,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum AiPlaylistStrategy {
    #[serde(rename = "ranked")]
    Ranked,
    #[serde(rename = "variety")]
    Variety,
    #[serde(rename = "discovery")]
    Discovery,
    #[serde(rename = "random")]
    Random,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionArtistEvidenceChartKind {
    #[serde(rename = "albums")]
    Albums,
    #[serde(rename = "singles")]
    Singles,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionStatus {
    #[serde(rename = "candidate")]
    Candidate,
    #[serde(rename = "wanted")]
    Wanted,
    #[serde(rename = "notForMe")]
    NotForMe,
    #[serde(rename = "needsReview")]
    NeedsReview,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum SetLibraryCompletionArtistVerificationStateRequestState {
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "paused")]
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionArtistVerificationItemSummaryProvider {
    #[serde(rename = "musicbrainz")]
    Musicbrainz,
    #[serde(rename = "discogs")]
    Discogs,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionArtistVerificationBatchState {
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "paused")]
    Paused,
    #[serde(rename = "completed")]
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum DeemixCredentialSource {
    #[serde(rename = "windowsCredentialManager")]
    WindowsCredentialManager,
    #[serde(rename = "none")]
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum DeemixAlbumMatchMatchLevel {
    #[serde(rename = "exact")]
    Exact,
    #[serde(rename = "likely")]
    Likely,
    #[serde(rename = "possible")]
    Possible,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum DeemixAlbumDownloadPhase {
    #[serde(rename = "metadata")]
    Metadata,
    #[serde(rename = "artwork")]
    Artwork,
    #[serde(rename = "downloading")]
    Downloading,
    #[serde(rename = "tagging")]
    Tagging,
    #[serde(rename = "complete")]
    Complete,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum DeemixDownloadQuality {
    #[serde(rename = "mp3_128")]
    Mp3128,
    #[serde(rename = "mp3_320")]
    Mp3320,
    #[serde(rename = "flac")]
    Flac,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum DiscogsCredentialStatusSource {
    #[serde(rename = "windowsCredentialManager")]
    WindowsCredentialManager,
    #[serde(rename = "none")]
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum ExternalDiscoveryEntity {
    #[serde(rename = "artist")]
    Artist,
    #[serde(rename = "album")]
    Album,
    #[serde(rename = "song")]
    Song,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum ExternalDiscoveryResponseSource {
    #[serde(rename = "MusicBrainz")]
    MusicBrainz,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionEvidenceSource {
    #[serde(rename = "billboard")]
    Billboard,
    #[serde(rename = "officialUk")]
    OfficialUk,
    #[serde(rename = "vgLista")]
    VgLista,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionConfidence {
    #[serde(rename = "best")]
    Best,
    #[serde(rename = "good")]
    Good,
    #[serde(rename = "needsReview")]
    NeedsReview,
    #[serde(rename = "low")]
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionCandidateCoverStatus {
    #[serde(rename = "checking")]
    Checking,
    #[serde(rename = "available")]
    Available,
    #[serde(rename = "unavailable")]
    Unavailable,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionCandidateCoverProvider {
    #[serde(rename = "musicbrainz")]
    Musicbrainz,
    #[serde(rename = "discogs")]
    Discogs,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionCandidateVerificationStatus {
    #[serde(rename = "unverified")]
    Unverified,
    #[serde(rename = "queued")]
    Queued,
    #[serde(rename = "checking")]
    Checking,
    #[serde(rename = "verified")]
    Verified,
    #[serde(rename = "noMatch")]
    NoMatch,
    #[serde(rename = "ambiguous")]
    Ambiguous,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionCandidateVerificationProvider {
    #[serde(rename = "musicbrainz")]
    Musicbrainz,
    #[serde(rename = "discogs")]
    Discogs,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionCandidateMusicbrainzVerificationStatus {
    #[serde(rename = "verified")]
    Verified,
    #[serde(rename = "noMatch")]
    NoMatch,
    #[serde(rename = "ambiguous")]
    Ambiguous,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionCandidateDiscogsVerificationStatus {
    #[serde(rename = "verified")]
    Verified,
    #[serde(rename = "noMatch")]
    NoMatch,
    #[serde(rename = "ambiguous")]
    Ambiguous,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum StartLibraryCompletionVerificationRequestScope {
    #[serde(rename = "candidate")]
    Candidate,
    #[serde(rename = "selection")]
    Selection,
    #[serde(rename = "campaign")]
    Campaign,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum PerformanceProbeOperationStatus {
    #[serde(rename = "ok")]
    Ok,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryUpdateKind {
    #[serde(rename = "new")]
    New,
    #[serde(rename = "changed")]
    Changed,
    #[serde(rename = "removed")]
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum CountryFlagDisplay {
    #[serde(rename = "flagAndName")]
    FlagAndName,
    #[serde(rename = "name")]
    Name,
    #[serde(rename = "flag")]
    Flag,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LeftSidebarMode {
    #[serde(rename = "expanded")]
    Expanded,
    #[serde(rename = "iconOnly")]
    IconOnly,
    #[serde(rename = "hidden")]
    Hidden,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum RightSidebarMode {
    #[serde(rename = "expanded")]
    Expanded,
    #[serde(rename = "hidden")]
    Hidden,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum DeemixDownloadOrganization {
    #[serde(rename = "flat_artist_album_year")]
    FlatArtistAlbumYear,
    #[serde(rename = "artist_album_year_folders")]
    ArtistAlbumYearFolders,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum MusicBrainzCacheStatusState {
    #[serde(rename = "available")]
    Available,
    #[serde(rename = "warning")]
    Warning,
    #[serde(rename = "unavailable")]
    Unavailable,
    #[serde(rename = "invalid")]
    Invalid,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum MusicBrainzArtistReleaseRowStatus {
    #[serde(rename = "owned")]
    Owned,
    #[serde(rename = "missing")]
    Missing,
    #[serde(rename = "excluded")]
    Excluded,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum MusicBrainzArtistExportRequestArtistLinkState {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "unverified")]
    Unverified,
    #[serde(rename = "verified")]
    Verified,
    #[serde(rename = "ignored")]
    Ignored,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum MusicBrainzArtistDiscographyResponseState {
    #[serde(rename = "available")]
    Available,
    #[serde(rename = "warning")]
    Warning,
    #[serde(rename = "unavailable")]
    Unavailable,
    #[serde(rename = "invalid")]
    Invalid,
    #[serde(rename = "notFound")]
    NotFound,
    #[serde(rename = "ignored")]
    Ignored,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum MusicBrainzArtistDiscographyResponseReleaseGroupSource {
    #[serde(rename = "cache")]
    Cache,
    #[serde(rename = "refreshed")]
    Refreshed,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum TextFilterOperator {
    #[serde(rename = "contains")]
    Contains,
    #[serde(rename = "doesNotContain")]
    DoesNotContain,
    #[serde(rename = "equals")]
    Equals,
    #[serde(rename = "startsWith")]
    StartsWith,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum BrowseSortDirection {
    #[serde(rename = "asc")]
    Asc,
    #[serde(rename = "desc")]
    Desc,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum ArtistTimelineMetric {
    #[serde(rename = "charts")]
    Charts,
    #[serde(rename = "albumScore")]
    AlbumScore,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum DiscoverySourceHealthState {
    #[serde(rename = "healthy")]
    Healthy,
    #[serde(rename = "stale")]
    Stale,
    #[serde(rename = "missing")]
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum DiscoverySourceHealthAction {
    #[serde(rename = "rebuild-chart-matches")]
    RebuildChartMatches,
    #[serde(rename = "open-imports")]
    OpenImports,
    #[serde(rename = "open-musicbrainz")]
    OpenMusicbrainz,
    #[serde(rename = "open-lastfm")]
    OpenLastfm,
    #[serde(rename = "open-covers")]
    OpenCovers,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum DiscoveryShelf {
    #[serde(rename = "anniversaries")]
    Anniversaries,
    #[serde(rename = "life-events")]
    LifeEvents,
    #[serde(rename = "charts")]
    Charts,
    #[serde(rename = "deep-cuts")]
    DeepCuts,
    #[serde(rename = "completion")]
    Completion,
    #[serde(rename = "recommendations")]
    Recommendations,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum DiscoveryShelfExplorerRequestEventType {
    #[serde(rename = "birthday")]
    Birthday,
    #[serde(rename = "memorial")]
    Memorial,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum DiscoveryRecommendationMode {
    #[serde(rename = "played")]
    Played,
    #[serde(rename = "loved")]
    Loved,
    #[serde(rename = "sonic")]
    Sonic,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum DiscoveryCompletionMode {
    #[serde(rename = "artist")]
    Artist,
    #[serde(rename = "album")]
    Album,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum DiscoveryChartSource {
    #[serde(rename = "billboard")]
    Billboard,
    #[serde(rename = "official-uk")]
    OfficialUk,
    #[serde(rename = "vg-lista")]
    VgLista,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum DiscoveryMixerSeedKind {
    #[serde(rename = "artist")]
    Artist,
    #[serde(rename = "album")]
    Album,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum DiscoveryMissionSortDirection {
    #[serde(rename = "asc")]
    Asc,
    #[serde(rename = "desc")]
    Desc,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum MusicToolSeverity {
    #[serde(rename = "high")]
    High,
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "low")]
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum MusicToolScope {
    #[serde(rename = "albums")]
    Albums,
    #[serde(rename = "tracks")]
    Tracks,
    #[serde(rename = "artists")]
    Artists,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum MusicToolProgressStatus {
    #[serde(rename = "starting")]
    Starting,
    #[serde(rename = "counting")]
    Counting,
    #[serde(rename = "loading")]
    Loading,
    #[serde(rename = "completed")]
    Completed,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum MusicToolFixDiffEntityType {
    #[serde(rename = "tracks")]
    Tracks,
    #[serde(rename = "albums")]
    Albums,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum MusicToolFixConfidence {
    #[serde(rename = "high")]
    High,
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "low")]
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum MusicToolFixHistoryEntryStatus {
    #[serde(rename = "applied")]
    Applied,
    #[serde(rename = "undone")]
    Undone,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum ChartViewMode {
    #[serde(rename = "table")]
    Table,
    #[serde(rename = "compact")]
    Compact,
    #[serde(rename = "grid")]
    Grid,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum MusicMapPointPrecision {
    #[serde(rename = "area")]
    Area,
    #[serde(rename = "country")]
    Country,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum WishListEntity {
    #[serde(rename = "artist")]
    Artist,
    #[serde(rename = "album")]
    Album,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum WishListEntity2 {
    #[serde(rename = "artist")]
    Artist,
    #[serde(rename = "album")]
    Album,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionArtistCandidateConfidence {
    #[serde(rename = "best")]
    Best,
    #[serde(rename = "good")]
    Good,
    #[serde(rename = "low")]
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionArtistCandidateVerificationStatus {
    #[serde(rename = "unverified")]
    Unverified,
    #[serde(rename = "queued")]
    Queued,
    #[serde(rename = "checking")]
    Checking,
    #[serde(rename = "verified")]
    Verified,
    #[serde(rename = "noMatch")]
    NoMatch,
    #[serde(rename = "ambiguous")]
    Ambiguous,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionArtistCandidateMusicbrainzVerificationStatus {
    #[serde(rename = "verified")]
    Verified,
    #[serde(rename = "noMatch")]
    NoMatch,
    #[serde(rename = "ambiguous")]
    Ambiguous,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionArtistCandidateDiscogsVerificationStatus {
    #[serde(rename = "verified")]
    Verified,
    #[serde(rename = "noMatch")]
    NoMatch,
    #[serde(rename = "ambiguous")]
    Ambiguous,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionArtistRequestChartKind {
    #[serde(rename = "albums")]
    Albums,
    #[serde(rename = "singles")]
    Singles,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionCoverEnrichmentProvider {
    #[serde(rename = "musicbrainz")]
    Musicbrainz,
    #[serde(rename = "discogs")]
    Discogs,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionAtlasCellSource {
    #[serde(rename = "billboard")]
    Billboard,
    #[serde(rename = "officialUk")]
    OfficialUk,
    #[serde(rename = "vgLista")]
    VgLista,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum ExternalDiscoveryEntity2 {
    #[serde(rename = "artist")]
    Artist,
    #[serde(rename = "album")]
    Album,
    #[serde(rename = "song")]
    Song,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum AiExternalDiscoveryPlanYearMeaning {
    #[serde(rename = "releaseYear")]
    ReleaseYear,
    #[serde(rename = "formedYear")]
    FormedYear,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionArtistVerificationItemSummaryState {
    #[serde(rename = "queued")]
    Queued,
    #[serde(rename = "checking")]
    Checking,
    #[serde(rename = "verified")]
    Verified,
    #[serde(rename = "noMatch")]
    NoMatch,
    #[serde(rename = "ambiguous")]
    Ambiguous,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionArtistEvidenceSource {
    #[serde(rename = "billboard")]
    Billboard,
    #[serde(rename = "officialUk")]
    OfficialUk,
    #[serde(rename = "vgLista")]
    VgLista,
    #[serde(rename = "tiISkuddet")]
    TiISkuddet,
    #[serde(rename = "norsktoppen")]
    Norsktoppen,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum LibraryCompletionCoverEnrichmentState {
    #[serde(rename = "checking")]
    Checking,
    #[serde(rename = "available")]
    Available,
    #[serde(rename = "unavailable")]
    Unavailable,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum MixtapeRole {
    #[serde(rename = "opener")]
    Opener,
    #[serde(rename = "builder")]
    Builder,
    #[serde(rename = "breather")]
    Breather,
    #[serde(rename = "closer")]
    Closer,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum AppUpdateInstallPhase {
    #[serde(rename = "downloading")]
    Downloading,
    #[serde(rename = "installing")]
    Installing,
    #[serde(rename = "restarting")]
    Restarting,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum JobState {
    #[serde(rename = "queued")]
    Queued,
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "pausing")]
    Pausing,
    #[serde(rename = "paused")]
    Paused,
    #[serde(rename = "cancelling")]
    Cancelling,
    #[serde(rename = "cancelled")]
    Cancelled,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "completed")]
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum MusicBrainzReleaseDecision {
    #[serde(rename = "not-in-scope")]
    NotInScope,
    #[serde(rename = "ignored")]
    Ignored,
    #[serde(rename = "include")]
    Include,
    #[serde(rename = "auto-not-official")]
    AutoNotOfficial,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum ListeningSource {
    #[serde(rename = "lastfm")]
    Lastfm,
    #[serde(rename = "listenbrainz")]
    Listenbrainz,
    #[serde(rename = "aurora")]
    Aurora,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub enum ListeningList {
    #[serde(rename = "topTracks")]
    TopTracks,
    #[serde(rename = "topAlbums")]
    TopAlbums,
    #[serde(rename = "topArtists")]
    TopArtists,
    #[serde(rename = "recent")]
    Recent,
    #[serde(rename = "rediscover")]
    Rediscover,
    #[serde(rename = "neverPlayedFavorites")]
    NeverPlayedFavorites,
    #[serde(rename = "leastPlayedAlbums")]
    LeastPlayedAlbums,
    #[serde(rename = "unmatched")]
    Unmatched,
}
