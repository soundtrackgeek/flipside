//! Name identity: every rule that decides whether two names are "the same".
//!
//! Each matching feature picks one level explicitly. From strictest to
//! loosest:
//!
//! | Level | Folds | Used by |
//! | --- | --- | --- |
//! | [`display_key`] | case, whitespace | genres, import history, folder sync |
//! | [`artist_key`] (SQL: [`artist_key_sql`]) | + typographic dashes; empty is `unknown` | Artists, artist filters, MusicBrainz overlay, portraits |
//! | [`strict_key`] | + NFKC, typographic apostrophes | Last.fm caches, biographies |
//! | [`loose_key`] | + accents, Nordic letters, `&`/`and`, punctuation | charts, Wish List, Artist Completion, Discogs, Discovery, Deemix, reviews |
//! | [`loose_artist_key`] | + leading "The", "and" | chart artist grouping |
//! | [`credit_keys`] | splits co-leads, drops "feat."/"with"/"x" guests | chart and artist credit matching |
//! | [`edition_title_key`] | + reissue decorations ("Remastered", "Deluxe Edition") | "already owned" album checks |
//!
//! Keys that are stored (chart entries, Wish List identities, Artist
//! Completion decisions, `idx_albums_artist_key`) make their level a data
//! contract: changing it needs a migration that rebuilds those keys.
//! `golden_corpus.csv` holds the expected behavior for tricky names; add a row
//! for every matching fix.

mod display;
mod loose;
mod strict;
mod title;

#[cfg(test)]
mod tests;

pub(crate) use display::{
    artist_key, artist_key_sql, catalog_artist_key_sql, artist_text_key, display_key, fold_dashes_sql,
};
pub(crate) use loose::{credit_keys, loose_artist_key, loose_key, strip_chart_country_suffix};
pub(crate) use strict::strict_key;
pub(crate) use title::edition_title_key;
