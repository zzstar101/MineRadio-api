use std::ops::{Deref, DerefMut};

use protobuf::Message;

use crate::{
    librespot_core::{Error, Session, SpotifyUri, date::Date, session::UserData},
    librespot_metadata::{
        MetadataError,
        availability::{AudioItemAvailability, Availabilities, UnavailabilityReason},
        restriction::Restrictions,
        util::impl_deref_wrapped,
    },
    librespot_protocol as protocol,
};

use super::file::AudioFiles;

pub type AudioItemResult = Result<AudioItem, Error>;

#[derive(Debug, Clone)]
pub struct AudioItem {
    pub uri: String,
    pub files: AudioFiles,
    pub availability: AudioItemAvailability,
    pub alternatives: Option<Tracks>,
}

#[derive(Debug, Clone, Default)]
pub struct Tracks(pub Vec<SpotifyUri>);

impl_deref_wrapped!(Tracks, Vec<SpotifyUri>);

impl AudioItem {
    pub async fn get_file(session: &Session, uri: SpotifyUri) -> AudioItemResult {
        match uri {
            SpotifyUri::Track { .. } => Self::get_track(session, uri).await,
            SpotifyUri::Episode { .. } => Self::get_episode(session, uri).await,
            _ => Err(Error::unavailable(MetadataError::NonPlayable)),
        }
    }

    async fn get_track(session: &Session, uri: SpotifyUri) -> AudioItemResult {
        let response = session.spclient().get_track_metadata(&uri).await?;
        let track = protocol::metadata::Track::parse_from_bytes(&response)?;

        if track.duration() <= 0 {
            return Err(Error::unavailable(MetadataError::InvalidDuration(
                track.duration(),
            )));
        }

        if track.explicit() && session.filter_explicit_content() {
            return Err(Error::unavailable(MetadataError::ExplicitContentFiltered));
        }

        let alternatives = track
            .alternative
            .iter()
            .filter_map(|track| SpotifyUri::try_from(track).ok())
            .collect::<Vec<_>>();

        let availability =
            if Date::now_utc() < Date::from_timestamp_ms(track.earliest_live_timestamp())? {
                Err(UnavailabilityReason::Embargo)
            } else {
                available_for_user(
                    &session.user_data(),
                    &track.availability.as_slice().try_into()?,
                    &track.restriction.as_slice().into(),
                )
            };

        Ok(Self {
            uri: uri.to_uri(),
            files: track.file.as_slice().into(),
            availability,
            alternatives: (!alternatives.is_empty()).then_some(Tracks(alternatives)),
        })
    }

    async fn get_episode(session: &Session, uri: SpotifyUri) -> AudioItemResult {
        let response = session.spclient().get_episode_metadata(&uri).await?;
        let episode = protocol::metadata::Episode::parse_from_bytes(&response)?;

        if episode.duration() <= 0 {
            return Err(Error::unavailable(MetadataError::InvalidDuration(
                episode.duration(),
            )));
        }

        if episode.explicit() && session.filter_explicit_content() {
            return Err(Error::unavailable(MetadataError::ExplicitContentFiltered));
        }

        Ok(Self {
            uri: uri.to_uri(),
            files: episode.audio.as_slice().into(),
            availability: available_for_user(
                &session.user_data(),
                &episode.availability.as_slice().try_into()?,
                &episode.restriction.as_slice().into(),
            ),
            alternatives: None,
        })
    }
}

fn allowed_for_user(user_data: &UserData, restrictions: &Restrictions) -> AudioItemAvailability {
    let country = &user_data.country;
    let user_catalogue = match user_data.attributes.get("catalogue") {
        Some(catalogue) => catalogue,
        None => "premium",
    };

    for premium_restriction in restrictions.iter().filter(|restriction| {
        restriction
            .catalogue_strs
            .iter()
            .any(|restricted_catalogue| restricted_catalogue == user_catalogue)
    }) {
        if let Some(allowed_countries) = &premium_restriction.countries_allowed {
            if allowed_countries.iter().any(|allowed| country == allowed) {
                return Ok(());
            } else {
                return Err(UnavailabilityReason::NotWhitelisted);
            }
        }

        if let Some(forbidden_countries) = &premium_restriction.countries_forbidden {
            if forbidden_countries
                .iter()
                .any(|forbidden| country == forbidden)
            {
                return Err(UnavailabilityReason::Blacklisted);
            } else {
                return Ok(());
            }
        }
    }

    Ok(())
}

fn available(availability: &Availabilities) -> AudioItemAvailability {
    if availability.is_empty() {
        return Ok(());
    }

    if !(availability
        .iter()
        .any(|availability| Date::now_utc() >= availability.start))
    {
        return Err(UnavailabilityReason::Embargo);
    }

    Ok(())
}

fn available_for_user(
    user_data: &UserData,
    availability: &Availabilities,
    restrictions: &Restrictions,
) -> AudioItemAvailability {
    available(availability)?;
    allowed_for_user(user_data, restrictions)?;
    Ok(())
}
