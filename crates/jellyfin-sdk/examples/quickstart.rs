//! Jellyfin SDK quickstart example.
//!
//! Common commands:
//!
//! - List library views (pick a `view id` for browsing):
//!   `cargo run --example quickstart -- --url http://localhost:8096 --token <token> --list-views`
//! - Continue watching (resume):
//!   `cargo run --example quickstart -- --url http://localhost:8096 --token <token> --list-resume`
//! - Remote subtitle search (then attach one by id):
//!   `cargo run --example quickstart -- --url http://localhost:8096 --token <token> --search-remote-subtitles --remote-subtitle-item-id <item_uuid> --remote-subtitle-language eng`
//!   `cargo run --example quickstart -- --url http://localhost:8096 --token <token> --attach-remote-subtitle <subtitle_id> --remote-subtitle-item-id <item_uuid>`

use clap::Parser;
use jellyfin_sdk::{
    JellyfinClient,
    api::{
        FiltersQuery, HlsPlaylistQuery, ItemImageRequest, ItemsQuery, LatestMediaQuery,
        MovieRecommendationsQuery, PlayOptions, RefreshItemQuery, ResumeItemsQuery,
        ScheduledTasksQuery, SearchHintsQuery, SessionSelector, SessionsQuery, SimilarItemsQuery,
        UserViewsQuery,
    },
    models::{
        BaseItemKind, ImageType, ItemField, ItemSortBy, PlayCommand, PlaybackInfoRequest,
        SortOrder, SubtitleFormat, SubtitleStreamQuery,
    },
};
use std::collections::{HashMap, HashSet};

#[derive(Parser, Debug)]
#[command(name = "quickstart", about = "Jellyfin SDK quickstart example")]
struct Args {
    /// Jellyfin base URL (e.g. http://localhost:8096). Can also be provided via env `JELLYFIN_URL`.
    #[arg(long)]
    url: Option<String>,

    /// Access token (can also be provided via env `JELLYFIN_TOKEN`).
    #[arg(long)]
    token: Option<String>,

    /// User id to use when the token isn't a user token (e.g. API key).
    #[arg(long)]
    user_id: Option<uuid::Uuid>,

    /// Username (can also be provided via env `JELLYFIN_USERNAME`)
    #[arg(long)]
    username: Option<String>,

    /// Password (can also be provided via env `JELLYFIN_PASSWORD`)
    #[arg(long)]
    password: Option<String>,

    /// List top-level views (`GET /UserViews`)
    #[arg(long)]
    list_views: bool,

    /// List latest media (`GET /Items/Latest`)
    #[arg(long)]
    list_latest: bool,

    /// List resume items (`GET /UserItems/Resume`)
    #[arg(long)]
    list_resume: bool,

    /// List genres (`GET /Genres`)
    #[arg(long)]
    list_genres: bool,

    /// List query filters (genres/tags) (`GET /Items/Filters2`)
    #[arg(long)]
    list_filters: bool,

    /// Search term for listing items.
    #[arg(long)]
    search: Option<String>,

    /// Search hint term (`GET /Search/Hints`).
    #[arg(long)]
    search_hints: Option<String>,

    /// Parent id for listing items (e.g. a view id from `--list-views`).
    #[arg(long)]
    parent_id: Option<uuid::Uuid>,

    /// Only list movie items.
    #[arg(long)]
    only_movies: bool,

    /// Filter list by genre name (repeatable).
    #[arg(long)]
    genre: Vec<String>,

    /// Filter list by genre id (repeatable).
    #[arg(long)]
    genre_id: Vec<uuid::Uuid>,

    /// Filter list by tag (repeatable).
    #[arg(long)]
    tag: Vec<String>,

    /// List similar items for an item id (`GET /Items/{itemId}/Similar`).
    #[arg(long)]
    similar_to: Option<uuid::Uuid>,

    /// List movie recommendations (`GET /Movies/Recommendations`).
    #[arg(long)]
    movie_recommendations: bool,

    /// Fetch playback info (`POST /Items/{itemId}/PlaybackInfo`).
    #[arg(long)]
    playback_info: Option<uuid::Uuid>,

    /// List intros for an item (`GET /Items/{itemId}/Intros`).
    #[arg(long)]
    intros: Option<uuid::Uuid>,

    /// List local trailers for an item (`GET /Items/{itemId}/LocalTrailers`).
    #[arg(long)]
    local_trailers: Option<uuid::Uuid>,

    /// List special features for an item (`GET /Items/{itemId}/SpecialFeatures`).
    #[arg(long)]
    special_features: Option<uuid::Uuid>,

    /// Download subtitles to `subtitle.<format>` (requires `--subtitle-item-id` + `--subtitle-index`).
    #[arg(long)]
    download_subtitle: bool,

    /// Item id for subtitle download.
    #[arg(long)]
    subtitle_item_id: Option<uuid::Uuid>,

    /// Subtitle stream index for subtitle download.
    #[arg(long)]
    subtitle_index: Option<i32>,

    /// Subtitle output format (e.g. srt, vtt).
    #[arg(long, default_value = "srt")]
    subtitle_format: String,

    /// List fallback fonts (`GET /FallbackFont/Fonts`).
    #[arg(long)]
    list_fallback_fonts: bool,

    /// Download a fallback font (`GET /FallbackFont/Fonts/{name}`).
    #[arg(long)]
    download_fallback_font: Option<String>,

    /// Search remote subtitles (`GET /Items/{itemId}/RemoteSearch/Subtitles/{language}`).
    #[arg(long)]
    search_remote_subtitles: bool,

    /// Item id for remote subtitle search / attach.
    #[arg(long)]
    remote_subtitle_item_id: Option<uuid::Uuid>,

    /// Language code for remote subtitle search (ISO-639-2, e.g. `eng`).
    #[arg(long, default_value = "eng")]
    remote_subtitle_language: String,

    /// Only return perfect matches during remote subtitle search.
    #[arg(long)]
    remote_subtitle_perfect_match: bool,

    /// Download a remote subtitle file (`GET /Providers/Subtitles/Subtitles/{subtitleId}`).
    #[arg(long)]
    download_remote_subtitle: Option<String>,

    /// Attach a remote subtitle to an item (`POST /Items/{itemId}/RemoteSearch/Subtitles/{subtitleId}`).
    #[arg(long)]
    attach_remote_subtitle: Option<String>,

    /// Output file path for `--download-remote-subtitle` and `--download-fallback-font`.
    #[arg(long)]
    output_path: Option<std::path::PathBuf>,

    /// Fetch and print the first lines of a master HLS playlist (`GET /Videos/{itemId}/master.m3u8`).
    #[arg(long)]
    hls_master: Option<uuid::Uuid>,

    /// Fetch and print the first lines of a variant HLS playlist (`GET /Videos/{itemId}/main.m3u8`).
    #[arg(long)]
    hls_main: Option<uuid::Uuid>,

    /// Fetch a playlist (`GET /Playlists/{playlistId}`).
    #[arg(long)]
    playlist: Option<uuid::Uuid>,

    /// List playlist items (`GET /Playlists/{playlistId}/Items`).
    #[arg(long)]
    playlist_items: Option<uuid::Uuid>,

    /// Compute tag counts from items and print in descending order.
    #[arg(long)]
    tag_counts: bool,

    /// How many tags to print when using `--tag-counts`.
    #[arg(long, default_value_t = 50)]
    tag_counts_top: usize,

    /// Get per-user data for an item (`GET /UserItems/{itemId}/UserData`).
    #[arg(long)]
    get_user_data: Option<uuid::Uuid>,

    /// Mark an item as played (`POST /UserItems/{itemId}/UserData` with Played=true).
    #[arg(long)]
    mark_played: Option<uuid::Uuid>,

    /// List user view grouping options (`GET /UserViews/GroupingOptions`).
    #[arg(long)]
    grouping_options: bool,

    /// Mark an item as favorite (`POST /UserFavoriteItems/{itemId}`).
    #[arg(long)]
    favorite: Option<uuid::Uuid>,

    /// Unmark an item as favorite (`DELETE /UserFavoriteItems/{itemId}`).
    #[arg(long)]
    unfavorite: Option<uuid::Uuid>,

    /// Like an item (`POST /UserItems/{itemId}/Rating?likes=true`).
    #[arg(long)]
    like: Option<uuid::Uuid>,

    /// Unlike an item (`POST /UserItems/{itemId}/Rating?likes=false`).
    #[arg(long)]
    unlike: Option<uuid::Uuid>,

    /// Delete user's rating (`DELETE /UserItems/{itemId}/Rating`).
    #[arg(long)]
    delete_rating: Option<uuid::Uuid>,

    /// List suggestions (`GET /Items/Suggestions`).
    #[arg(long)]
    list_suggestions: bool,

    /// Get root folder (`GET /Items/Root`).
    #[arg(long)]
    root_folder: bool,

    /// List all parents of an item (`GET /Items/{itemId}/Ancestors`).
    #[arg(long)]
    ancestors: Option<uuid::Uuid>,

    /// Download item media (`GET /Items/{itemId}/Download`).
    #[arg(long)]
    download_item: Option<uuid::Uuid>,

    /// Download original file stream (`GET /Items/{itemId}/File`).
    #[arg(long)]
    download_file: Option<uuid::Uuid>,

    /// Issue a HEAD request for a video stream (`HEAD /Videos/{itemId}/stream`).
    #[arg(long)]
    head_video_stream: Option<uuid::Uuid>,

    /// Container extension for video stream requests (e.g. `mp4`).
    #[arg(long)]
    video_container: Option<String>,

    /// List virtual folders (`GET /Library/VirtualFolders`).
    #[arg(long)]
    list_virtual_folders: bool,

    /// List scheduled tasks (`GET /ScheduledTasks`).
    #[arg(long)]
    list_scheduled_tasks: bool,

    /// Queue a metadata refresh for an item (`POST /Items/{itemId}/Refresh`).
    #[arg(long)]
    refresh_item: Option<uuid::Uuid>,

    /// Dump application configuration (`GET /System/Configuration`).
    #[arg(long)]
    get_configuration: bool,

    /// Dump default metadata options (`GET /System/Configuration/MetadataOptions/Default`).
    #[arg(long)]
    get_default_metadata_options: bool,

    /// Fetch a named configuration blob (`GET /System/Configuration/{key}`).
    #[arg(long)]
    get_named_configuration: Option<String>,

    /// List API keys (`GET /Auth/Keys`).
    #[arg(long)]
    list_api_keys: bool,

    /// Create an API key (`POST /Auth/Keys?app=...`).
    #[arg(long)]
    create_api_key: Option<String>,

    /// Revoke an API key (`DELETE /Auth/Keys/{key}`).
    #[arg(long)]
    revoke_api_key: Option<String>,

    /// List installed plugins (`GET /Plugins`).
    #[arg(long)]
    list_plugins: bool,

    /// List image infos for an item (`GET /Items/{itemId}/Images`).
    #[arg(long)]
    item_image_infos: Option<uuid::Uuid>,

    /// Download a user's profile image (`GET /UserImage`).
    #[arg(long)]
    user_image: Option<uuid::Uuid>,

    /// Output file path for `--download-item`.
    #[arg(long)]
    download_path: Option<std::path::PathBuf>,

    /// Fetch an item's details (`GET /Items/{itemId}`)
    #[arg(long)]
    item_id: Option<uuid::Uuid>,

    /// List sessions controllable by the logged-in user
    #[arg(long)]
    list_sessions: bool,

    /// Download a primary image to `jellyfin_primary.jpg`
    #[arg(long)]
    download_image: bool,

    /// Remote play to a controllable session
    #[arg(long)]
    remote_play: bool,

    /// Target a specific session id for remote control
    #[arg(long)]
    session_id: Option<String>,

    /// Narrow down auto-selected session by device name (substring match)
    #[arg(long)]
    session_device_name: Option<String>,

    /// Narrow down auto-selected session by client name (substring match)
    #[arg(long)]
    session_client: Option<String>,

    /// Seek after remote play; supports `SS`, `MM:SS`, `HH:MM:SS`
    #[arg(long)]
    seek: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let username_arg_provided = args.username.is_some();
    let password_arg_provided = args.password.is_some();

    let base_url = args
        .url
        .or_else(|| std::env::var("JELLYFIN_URL").ok())
        .unwrap_or_else(|| "http://localhost:8096".to_owned());
    let token = args.token.or_else(|| std::env::var("JELLYFIN_TOKEN").ok());
    let username = args
        .username
        .or_else(|| std::env::var("JELLYFIN_USERNAME").ok());
    let password = args
        .password
        .or_else(|| std::env::var("JELLYFIN_PASSWORD").ok());

    let client = JellyfinClient::builder(&base_url)?
        .client_name("jellyfin-sdk-example")
        .device_name("rust")
        .build()?;

    let info = client.system().get_public_info().await?;
    println!("server={:?} version={:?}", info.server_name, info.version);

    let ping = client.system().ping().await?;
    println!("ping={ping}");

    let token_provided = token.is_some();
    let mut token_set = false;
    if let Some(token) = token {
        client.set_token(token);
        println!("token_set=true");
        token_set = true;
    } else if let (Some(username), Some(password)) = (username, password) {
        let auth = client
            .user()
            .authenticate_by_name_and_set_token(username, password)
            .await?;
        println!("access_token_present={}", auth.access_token.is_some());
        token_set = auth.access_token.is_some();
    }

    let mut user_id = args.user_id;

    match client.user().me().await {
        Ok(me) => {
            println!("me_id={:?} me_name={:?}", me.id, me.name);
            if user_id.is_none() {
                user_id = me.id;
            }
        }
        Err(jellyfin_sdk::Error::Api { status, body, .. }) => {
            if status == reqwest::StatusCode::BAD_REQUEST {
                if token_provided && user_id.is_none() {
                    println!(
                        "token is not owned by a user; pass --user-id <uuid> to run user-scoped APIs"
                    );
                }
            } else if token_provided || username_arg_provided || password_arg_provided {
                return Err(anyhow::anyhow!(
                    "failed to fetch current user: status={status} body={body}"
                ));
            }
        }
        Err(err) => {
            if token_provided || username_arg_provided || password_arg_provided {
                return Err(err.into());
            }
        }
    }

    if let Some(user_id) = user_id {
        if args.list_views {
            let views = client
                .user_views()
                .get_user_views(UserViewsQuery::new().user_id(user_id).include_hidden(false))
                .await?;
            println!("views={:?}", views.total_record_count);
            for v in views.items.iter().take(5) {
                println!("view id={:?} kind={:?} name={:?}", v.id, v.kind, v.name);
            }
        }

        if args.grouping_options {
            let opts = client
                .user_views()
                .get_grouping_options(Some(user_id))
                .await?;
            println!("grouping_options={}", opts.len());
            for o in opts.iter().take(20) {
                println!("option id={:?} name={:?}", o.id, o.name);
            }
        }

        let mut pager = client
            .items()
            .pager({
                let mut q = ItemsQuery::new()
                    .user_id(user_id)
                    .recursive(true)
                    .field(ItemField::Overview)
                    .sort_by(ItemSortBy::SortName)
                    .sort_order(SortOrder::Ascending);

                if let Some(parent_id) = args.parent_id {
                    q = q.parent_id(parent_id);
                }
                if let Some(search) = args.search.clone() {
                    q = q.search_term(search);
                }
                if args.only_movies {
                    q = q.include_item_type(BaseItemKind::Movie);
                }
                for g in args.genre.iter().cloned() {
                    q = q.genre(g);
                }
                for id in args.genre_id.iter().copied() {
                    q = q.genre_id(id);
                }
                for t in args.tag.iter().cloned() {
                    q = q.tag(t);
                }
                q
            })
            .limit(25);

        if let Some(page) = pager.next_page().await? {
            println!(
                "items_page_len={} total={:?}",
                page.items.len(),
                page.total_record_count
            );
            for item in page.items.iter().take(5) {
                println!(
                    "item id={:?} kind={:?} name={:?}",
                    item.id, item.kind, item.name
                );
            }

            if args.download_image
                && let Some(item_id) = page.items.iter().find_map(|i| i.id)
            {
                client
                    .items()
                    .download_image_to_file(
                        item_id,
                        ImageType::Primary,
                        ItemImageRequest::new().max_width(512),
                        "jellyfin_primary.jpg",
                    )
                    .await?;
                println!("downloaded jellyfin_primary.jpg");
            }
        }

        if args.list_genres {
            let mut q = jellyfin_sdk::api::GenresQuery::new().user_id(user_id);
            if let Some(parent_id) = args.parent_id {
                q = q.parent_id(parent_id);
            }
            if let Some(search) = args.search.clone() {
                q = q.search_term(search);
            }

            let genres = client.genres().get_genres(q.limit(50)).await?;
            println!(
                "genres len={} total={:?}",
                genres.items.len(),
                genres.total_record_count
            );
            for g in genres.items.iter().take(10) {
                println!("genre id={:?} name={:?}", g.id, g.name);
            }
        }

        if args.list_filters {
            let mut q = FiltersQuery::new().user_id(user_id).recursive(true);
            if let Some(parent_id) = args.parent_id {
                q = q.parent_id(parent_id);
            }
            if args.only_movies {
                q = q.is_movie(true);
            }

            let filters = client.filters().get_query_filters(q).await?;
            println!(
                "filters genres={:?} tags={:?}",
                filters.genres.as_ref().map(|v| v.len()),
                filters.tags.as_ref().map(|v| v.len())
            );
            if let Some(tags) = filters.tags.as_ref() {
                for tag in tags.iter().take(20) {
                    println!("tag={tag:?}");
                }
            }
        }

        if let Some(term) = args.search_hints.clone() {
            let hints = client
                .search()
                .get_search_hints(SearchHintsQuery::new(term).user_id(user_id).limit(10))
                .await?;
            println!(
                "search_hints len={} total={:?}",
                hints.search_hints.len(),
                hints.total_record_count
            );
            for h in hints.search_hints.iter().take(10) {
                println!(
                    "hint id={:?} kind={:?} name={:?} year={:?}",
                    h.id, h.kind, h.name, h.production_year
                );
            }
        }

        if let Some(item_id) = args.similar_to {
            let similar = client
                .items()
                .get_similar_items(
                    item_id,
                    SimilarItemsQuery::new()
                        .user_id(user_id)
                        .limit(10)
                        .field(ItemField::Overview),
                )
                .await?;
            println!(
                "similar len={} total={:?}",
                similar.items.len(),
                similar.total_record_count
            );
            for item in similar.items.iter().take(10) {
                println!(
                    "similar id={:?} kind={:?} name={:?} year={:?}",
                    item.id, item.kind, item.name, item.production_year
                );
            }
        }

        if args.movie_recommendations {
            let mut q = MovieRecommendationsQuery::new()
                .user_id(user_id)
                .category_limit(5)
                .item_limit(8)
                .field(ItemField::Overview);
            if let Some(parent_id) = args.parent_id {
                q = q.parent_id(parent_id);
            }

            let recs = client.movies().get_recommendations(q).await?;
            println!("movie_recommendations categories={}", recs.len());
            for r in recs.iter().take(10) {
                println!(
                    "recommendation type={:?} baseline={:?} items={}",
                    r.recommendation_type,
                    r.baseline_item_name,
                    r.items.len()
                );
                for item in r.items.iter().take(3) {
                    println!(
                        "  item id={:?} kind={:?} name={:?} year={:?}",
                        item.id, item.kind, item.name, item.production_year
                    );
                }
            }
        }

        if let Some(item_id) = args.playback_info {
            let info = client
                .items()
                .post_playback_info(item_id, PlaybackInfoRequest::new().user_id(user_id))
                .await?;
            println!(
                "playback_info media_sources={} play_session_id={:?} error_code={:?}",
                info.media_sources.len(),
                info.play_session_id,
                info.error_code
            );
            if let Some(ms) = info.media_sources.first() {
                println!(
                    "media_source id={:?} container={:?} bitrate={:?} direct_play={:?} transcode_url={:?}",
                    ms.id, ms.container, ms.bitrate, ms.supports_direct_play, ms.transcoding_url
                );
            }
        }

        if let Some(item_id) = args.intros {
            let intros = client
                .user_library()
                .get_intros(item_id, Some(user_id))
                .await?;
            println!(
                "intros items={} total={:?}",
                intros.items.len(),
                intros.total_record_count
            );
            for it in intros.items.iter().take(10) {
                println!(
                    "  intro id={:?} kind={:?} name={:?} runtime_ticks={:?}",
                    it.id, it.kind, it.name, it.run_time_ticks
                );
            }
        }

        if let Some(item_id) = args.local_trailers {
            let trailers = client
                .user_library()
                .get_local_trailers(item_id, Some(user_id))
                .await?;
            println!("local_trailers={}", trailers.len());
            for it in trailers.iter().take(10) {
                println!(
                    "  trailer id={:?} kind={:?} name={:?} runtime_ticks={:?}",
                    it.id, it.kind, it.name, it.run_time_ticks
                );
            }
        }

        if let Some(item_id) = args.special_features {
            let features = client
                .user_library()
                .get_special_features(item_id, Some(user_id))
                .await?;
            println!("special_features={}", features.len());
            for it in features.iter().take(10) {
                println!(
                    "  feature id={:?} kind={:?} name={:?} runtime_ticks={:?}",
                    it.id, it.kind, it.name, it.run_time_ticks
                );
            }
        }

        if args.download_subtitle {
            let Some(item_id) = args.subtitle_item_id else {
                anyhow::bail!("--download-subtitle requires --subtitle-item-id");
            };
            let Some(index) = args.subtitle_index else {
                anyhow::bail!("--download-subtitle requires --subtitle-index");
            };

            let playback = client
                .items()
                .post_playback_info(item_id, PlaybackInfoRequest::new().user_id(user_id))
                .await?;

            let format: SubtitleFormat = args.subtitle_format.as_str().into();
            let file_name = format!("subtitle.{}", format.as_str());

            client
                .subtitles()
                .download_subtitle_from_playback_info_to_file(
                    item_id,
                    &playback,
                    index,
                    format,
                    SubtitleStreamQuery::new(),
                    &file_name,
                )
                .await?;

            println!("downloaded {file_name}");
        }

        if let Some(item_id) = args.hls_master {
            let playback = client
                .items()
                .post_playback_info(item_id, PlaybackInfoRequest::new().user_id(user_id))
                .await?;
            let Some(media_source_id) = playback.media_sources.first().and_then(|s| s.id.clone())
            else {
                anyhow::bail!("playback info contains no media sources");
            };
            let Some(play_session_id) = playback.play_session_id.clone() else {
                anyhow::bail!("playback info contains no play_session_id");
            };

            let resp = client
                .dynamic_hls()
                .get_master_video_playlist(
                    item_id,
                    media_source_id,
                    HlsPlaylistQuery::new()
                        .play_session_id(play_session_id)
                        .device_id(client.device_id().to_owned()),
                )
                .await?;
            let text = resp.text().await?;
            print_first_lines("master.m3u8", &text, 30);
        }

        if let Some(item_id) = args.hls_main {
            let playback = client
                .items()
                .post_playback_info(item_id, PlaybackInfoRequest::new().user_id(user_id))
                .await?;
            let Some(media_source_id) = playback.media_sources.first().and_then(|s| s.id.clone())
            else {
                anyhow::bail!("playback info contains no media sources");
            };
            let Some(play_session_id) = playback.play_session_id.clone() else {
                anyhow::bail!("playback info contains no play_session_id");
            };

            let resp = client
                .dynamic_hls()
                .get_variant_video_playlist(
                    item_id,
                    HlsPlaylistQuery::new()
                        .media_source_id(media_source_id)
                        .play_session_id(play_session_id)
                        .device_id(client.device_id().to_owned()),
                )
                .await?;
            let text = resp.text().await?;
            print_first_lines("main.m3u8", &text, 30);
        }

        if let Some(playlist_id) = args.playlist_items {
            let items = client
                .playlists()
                .get_playlist_items(
                    playlist_id,
                    jellyfin_sdk::api::PlaylistItemsQuery::new()
                        .user_id(user_id)
                        .enable_images(false)
                        .enable_user_data(true)
                        .limit(50),
                )
                .await?;
            println!(
                "playlist_items count={} total={:?}",
                items.items.len(),
                items.total_record_count
            );
            for it in items.items.iter().take(10) {
                println!(
                    "  item id={:?} kind={:?} name={:?}",
                    it.id, it.kind, it.name
                );
            }
        }

        if args.tag_counts {
            let mut q = ItemsQuery::new()
                .user_id(user_id)
                .recursive(true)
                .field(ItemField::Tags);

            if let Some(parent_id) = args.parent_id {
                q = q.parent_id(parent_id);
            }
            if args.only_movies {
                q = q.include_item_type(BaseItemKind::Movie);
            }

            let mut pager = client.items().pager(q).limit(500);
            let mut counts: HashMap<String, u64> = HashMap::new();

            while let Some(page) = pager.next_page().await? {
                for item in page.items {
                    let Some(tags) = item.tags else { continue };
                    let unique: HashSet<String> = tags.into_iter().collect();
                    for tag in unique {
                        *counts.entry(tag).or_insert(0) += 1;
                    }
                }
            }

            let mut ranked: Vec<(String, u64)> = counts.into_iter().collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

            println!("tag_counts total_unique={}", ranked.len());
            for (tag, count) in ranked.into_iter().take(args.tag_counts_top) {
                println!("{count}\t{tag}");
            }
        }

        if let Some(item_id) = args.get_user_data {
            let data = client.items().get_user_data(item_id, Some(user_id)).await?;
            println!(
                "user_data item_id={:?} played={:?} position_ticks={:?} played_pct={:?} favorite={:?} play_count={:?} rating={:?}",
                data.item_id,
                data.played,
                data.playback_position_ticks,
                data.played_percentage,
                data.is_favorite,
                data.play_count,
                data.rating
            );
        }

        if let Some(item_id) = args.mark_played {
            let updated = client
                .items()
                .update_user_data(
                    item_id,
                    Some(user_id),
                    jellyfin_sdk::models::UpdateUserItemData::new().played(true),
                )
                .await?;
            println!(
                "marked played: played={:?} play_count={:?}",
                updated.played, updated.play_count
            );
        }

        if let Some(item_id) = args.favorite {
            let updated = client
                .user_library()
                .mark_favorite(item_id, Some(user_id))
                .await?;
            println!("favorite: is_favorite={:?}", updated.is_favorite);
        }

        if let Some(item_id) = args.unfavorite {
            let updated = client
                .user_library()
                .unmark_favorite(item_id, Some(user_id))
                .await?;
            println!("unfavorite: is_favorite={:?}", updated.is_favorite);
        }

        if let Some(item_id) = args.like {
            let updated = client
                .user_library()
                .update_item_like(item_id, Some(user_id), true)
                .await?;
            println!("like: likes={:?}", updated.likes);
        }

        if let Some(item_id) = args.unlike {
            let updated = client
                .user_library()
                .update_item_like(item_id, Some(user_id), false)
                .await?;
            println!("unlike: likes={:?}", updated.likes);
        }

        if let Some(item_id) = args.delete_rating {
            let updated = client
                .user_library()
                .delete_item_rating(item_id, Some(user_id))
                .await?;
            println!(
                "delete_rating: rating={:?} likes={:?}",
                updated.rating, updated.likes
            );
        }

        if args.list_suggestions {
            let suggestions = client
                .suggestions()
                .get_suggestions(
                    jellyfin_sdk::api::SuggestionsQuery::new()
                        .user_id(user_id)
                        .media_type(jellyfin_sdk::models::MediaType::Video)
                        .enable_total_record_count(true)
                        .limit(20),
                )
                .await?;
            println!(
                "suggestions len={} total={:?}",
                suggestions.items.len(),
                suggestions.total_record_count
            );
            for item in suggestions.items.iter().take(10) {
                println!(
                    "suggestion id={:?} kind={:?} name={:?}",
                    item.id, item.kind, item.name
                );
            }
        }

        if args.root_folder {
            let root = client.user_library().get_root_folder(Some(user_id)).await?;
            println!(
                "root_folder id={:?} kind={:?} name={:?}",
                root.id, root.kind, root.name
            );
        }

        if let Some(item_id) = args.ancestors {
            let ancestors = client.items().get_ancestors(item_id, Some(user_id)).await?;
            println!("ancestors len={}", ancestors.len());
            for a in ancestors {
                println!("ancestor id={:?} kind={:?} name={:?}", a.id, a.kind, a.name);
            }
        }

        if args.list_sessions {
            let sessions = client
                .sessions()
                .get_sessions(SessionsQuery::new().controllable_by_user_id(user_id))
                .await?;
            println!("sessions={}", sessions.len());
            for s in sessions.iter().take(3) {
                println!(
                    "session id={:?} device={:?} now_playing={:?}",
                    s.id,
                    s.device_name,
                    s.now_playing_item.as_ref().and_then(|i| i.name.clone())
                );
            }
        }

        if args.list_latest {
            let latest = client
                .user_library()
                .get_latest_media(
                    LatestMediaQuery::new()
                        .user_id(user_id)
                        .limit(10)
                        .group_items(false),
                )
                .await?;
            println!("latest={}", latest.len());
            for item in latest.iter().take(5) {
                println!(
                    "latest id={:?} kind={:?} name={:?} year={:?}",
                    item.id, item.kind, item.name, item.production_year
                );
            }
        }

        if args.list_resume {
            let resume = client
                .user_library()
                .get_resume_items(ResumeItemsQuery::new().user_id(user_id).limit(10))
                .await?;
            println!(
                "resume len={} total={:?}",
                resume.items.len(),
                resume.total_record_count
            );
            for item in resume.items.iter().take(5) {
                println!(
                    "resume id={:?} kind={:?} name={:?} pos={:?}",
                    item.id,
                    item.kind,
                    item.name,
                    item.run_time_ticks.map(|_| "ticks_present")
                );
            }
        }

        if let Some(item_id) = args.item_id {
            let item = client
                .user_library()
                .get_item(item_id, Some(user_id))
                .await?;
            println!(
                "item id={:?} kind={:?} name={:?} year={:?} genres={:?} tags={:?} taglines={:?} studios={:?} provider_ids={:?}",
                item.id,
                item.kind,
                item.name,
                item.production_year,
                item.genres,
                item.tags,
                item.taglines,
                item.studios,
                item.provider_ids
            );
        }

        if args.remote_play {
            let session = if let Some(id) = args.session_id.clone() {
                Some(client.sessions().remote(id))
            } else {
                let mut selector = SessionSelector::new().require_media_control(true);
                if let Some(device_name) = args.session_device_name.clone() {
                    selector = selector.device_name_contains(device_name);
                }
                if let Some(client_name) = args.session_client.clone() {
                    selector = selector.client_contains(client_name);
                }

                client
                    .sessions()
                    .first_controllable_session_matching(user_id, selector)
                    .await?
            };

            let Some(session) = session else {
                println!("no controllable session found");
                return Ok(());
            };

            let Some(item_id) = client
                .items()
                .pager(ItemsQuery::new().user_id(user_id).recursive(true))
                .limit(1)
                .next_page()
                .await?
                .and_then(|p| p.items.into_iter().find_map(|i| i.id))
            else {
                println!("no item found to play");
                return Ok(());
            };

            session
                .play(PlayOptions::new(PlayCommand::PlayNow, vec![item_id]))
                .await?;
            println!("remote play sent to session_id={}", session.id());

            if let Some(timecode) = args.seek.clone() {
                session.seek_timecode(&timecode).await?;
                println!("seek sent to {}", timecode);
            }
        }
    }

    if token_set {
        if args.list_fallback_fonts {
            let fonts = client.subtitles().get_fallback_font_list().await?;
            println!("fallback_fonts={}", fonts.len());
            for f in fonts.iter().take(50) {
                println!(
                    "font name={:?} size={:?} created={:?} modified={:?}",
                    f.name, f.size, f.date_created, f.date_modified
                );
            }
        }

        if let Some(name) = args.download_fallback_font.clone() {
            let out = args
                .output_path
                .clone()
                .unwrap_or_else(|| std::path::PathBuf::from(&name));
            client
                .subtitles()
                .download_fallback_font_to_file(name, &out)
                .await?;
            println!("downloaded fallback font to {}", out.display());
        }

        if args.search_remote_subtitles {
            let Some(item_id) = args.remote_subtitle_item_id else {
                anyhow::bail!("--search-remote-subtitles requires --remote-subtitle-item-id");
            };
            let perfect_match = if args.remote_subtitle_perfect_match {
                Some(true)
            } else {
                None
            };

            let results = client
                .subtitles()
                .search_remote_subtitles(item_id, &args.remote_subtitle_language, perfect_match)
                .await?;
            println!("remote_subtitles={}", results.len());
            for s in results.iter().take(50) {
                println!(
                    "subtitle id={:?} provider={:?} lang={:?} format={:?} downloads={:?} rating={:?} name={:?}",
                    s.id,
                    s.provider_name,
                    s.three_letter_iso_language_name,
                    s.format,
                    s.download_count,
                    s.community_rating,
                    s.name
                );
            }
        }

        if let Some(subtitle_id) = args.download_remote_subtitle.clone() {
            let out = args
                .output_path
                .clone()
                .unwrap_or_else(|| "remote_subtitle.bin".into());
            client
                .subtitles()
                .download_remote_subtitle_to_file(subtitle_id, &out)
                .await?;
            println!("downloaded remote subtitle to {}", out.display());
        }

        if let Some(subtitle_id) = args.attach_remote_subtitle.clone() {
            let Some(item_id) = args.remote_subtitle_item_id else {
                anyhow::bail!("--attach-remote-subtitle requires --remote-subtitle-item-id");
            };
            client
                .subtitles()
                .download_remote_subtitle(item_id, subtitle_id)
                .await?;
            println!("attached remote subtitle to item_id={item_id}");
        }

        if args.list_virtual_folders {
            let folders = client.library_structure().get_virtual_folders().await?;
            for folder in folders {
                println!(
                    "virtual_folder name={:?} collection_type={:?} locations={:?}",
                    folder.name, folder.collection_type, folder.locations
                );
            }
        }

        if args.list_scheduled_tasks {
            let tasks = client
                .scheduled_tasks()
                .get_tasks(ScheduledTasksQuery::new())
                .await?;
            for task in tasks {
                println!(
                    "task id={:?} name={:?} state={:?} progress={:?} category={:?}",
                    task.id, task.name, task.state, task.current_progress_percentage, task.category
                );
            }
        }

        if let Some(item_id) = args.refresh_item {
            client
                .items()
                .refresh_item(item_id, RefreshItemQuery::new())
                .await?;
            println!("refresh queued for item_id={item_id}");
        }

        if args.get_configuration {
            let config = client.configuration().get_configuration().await?;
            println!("{}", serde_json::to_string_pretty(&config)?);
        }

        if args.get_default_metadata_options {
            let opts = client
                .configuration()
                .get_default_metadata_options()
                .await?;
            println!("{}", serde_json::to_string_pretty(&opts)?);
        }

        if let Some(key) = args.get_named_configuration.clone() {
            let resp = client.configuration().get_named_configuration(key).await?;
            let bytes = resp.bytes().await?;
            println!("named configuration bytes={}", bytes.len());
        }

        if args.list_api_keys {
            let keys = client.api_key().get_keys().await?;
            println!("api_keys count={}", keys.items.len());
            for k in keys.items {
                let masked = mask_token(k.access_token.as_deref());
                println!(
                    "api_key app={:?} device={:?} user_id={:?} active={:?} token={masked}",
                    k.app_name, k.device_name, k.user_id, k.is_active
                );
            }
        }

        if let Some(app) = args.create_api_key.clone() {
            client.api_key().create_key(app).await?;
            println!("api key created (list keys to view it)");
        }

        if let Some(key) = args.revoke_api_key.clone() {
            client.api_key().revoke_key(key).await?;
            println!("api key revoked");
        }

        if args.list_plugins {
            let plugins = client.plugins().get_plugins().await?;
            println!("plugins count={}", plugins.len());
            for p in plugins {
                println!(
                    "plugin id={:?} name={:?} version={:?} status={:?}",
                    p.id, p.name, p.version, p.status
                );
            }
        }

        if let Some(item_id) = args.item_image_infos {
            let infos = client.items().get_image_infos(item_id).await?;
            println!("image_infos count={}", infos.len());
            for info in infos.iter().take(20) {
                println!(
                    "  type={:?} index={:?} tag={:?} w={:?} h={:?}",
                    info.image_type, info.image_index, info.image_tag, info.width, info.height
                );
            }
        }

        if let Some(target_user_id) = args.user_image {
            client
                .images()
                .download_user_image_to_file(
                    target_user_id,
                    jellyfin_sdk::api::UserImageRequest::new(),
                    "jellyfin_user_image",
                )
                .await?;
            println!("downloaded jellyfin_user_image");
        }

        if let Some(playlist_id) = args.playlist {
            let playlist = client.playlists().get_playlist(playlist_id).await?;
            println!(
                "playlist open_access={:?} item_ids={} shares={}",
                playlist.open_access,
                playlist.item_ids.len(),
                playlist.shares.len()
            );
        }

        if let Some(item_id) = args.download_item {
            let out = args
                .download_path
                .clone()
                .unwrap_or_else(|| "jellyfin_download.bin".into());
            client.items().download_item_to_file(item_id, &out).await?;
            println!("downloaded {}", out.display());
        }

        if let Some(item_id) = args.download_file {
            let out = args
                .download_path
                .clone()
                .unwrap_or_else(|| "jellyfin_file.bin".into());
            client.items().download_file_to_path(item_id, &out).await?;
            println!("downloaded {}", out.display());
        }

        if let Some(item_id) = args.head_video_stream {
            let q = jellyfin_sdk::models::VideoStreamQuery::new().static_stream(true);
            let resp = if let Some(container) = args.video_container.as_deref() {
                client
                    .videos()
                    .head_video_stream_by_container(item_id, container, q)
                    .await?
            } else {
                client.videos().head_video_stream(item_id, q).await?
            };

            println!(
                "head_video_stream status={} content_length={:?} content_type={:?}",
                resp.status(),
                resp.content_length(),
                resp.headers().get(reqwest::header::CONTENT_TYPE)
            );
        }
    }

    let server_scoped_requested = args.list_fallback_fonts
        || args.download_fallback_font.is_some()
        || args.search_remote_subtitles
        || args.download_remote_subtitle.is_some()
        || args.attach_remote_subtitle.is_some()
        || args.download_item.is_some()
        || args.download_file.is_some()
        || args.head_video_stream.is_some()
        || args.list_virtual_folders
        || args.list_scheduled_tasks
        || args.refresh_item.is_some()
        || args.get_configuration
        || args.get_default_metadata_options
        || args.get_named_configuration.is_some()
        || args.list_api_keys
        || args.create_api_key.is_some()
        || args.revoke_api_key.is_some()
        || args.list_plugins
        || args.item_image_infos.is_some()
        || args.user_image.is_some()
        || args.playlist.is_some();

    let user_scoped_requested = args.list_views
        || args.list_latest
        || args.list_resume
        || args.list_genres
        || args.list_filters
        || args.search.is_some()
        || args.search_hints.is_some()
        || args.similar_to.is_some()
        || args.movie_recommendations
        || args.playback_info.is_some()
        || args.intros.is_some()
        || args.local_trailers.is_some()
        || args.special_features.is_some()
        || args.download_subtitle
        || args.item_id.is_some()
        || args.list_sessions
        || args.remote_play
        || args.download_image
        || args.hls_master.is_some()
        || args.hls_main.is_some()
        || args.playlist_items.is_some()
        || args.tag_counts
        || args.get_user_data.is_some()
        || args.mark_played.is_some()
        || args.favorite.is_some()
        || args.unfavorite.is_some()
        || args.like.is_some()
        || args.unlike.is_some()
        || args.delete_rating.is_some()
        || args.list_suggestions
        || args.root_folder
        || args.ancestors.is_some()
        || args.grouping_options;

    let any_requested = server_scoped_requested || user_scoped_requested;

    if !token_set && any_requested {
        println!("auth required: pass --token or --username/--password");
    } else if user_id.is_none() && user_scoped_requested {
        println!(
            "user-scoped APIs require a user id; pass --user-id <uuid> if your token isn't a user token"
        );
    }

    Ok(())
}

fn mask_token(token: Option<&str>) -> String {
    let Some(token) = token else {
        return "<none>".to_owned();
    };
    if token.len() <= 8 {
        return "<redacted>".to_owned();
    }
    format!("{}...{}", &token[..4], &token[token.len() - 4..])
}

fn print_first_lines(name: &str, text: &str, max_lines: usize) {
    println!("--- {name} (first {max_lines} lines) ---");
    for (i, line) in text.lines().take(max_lines).enumerate() {
        println!("{:>3} {line}", i + 1);
    }
}
