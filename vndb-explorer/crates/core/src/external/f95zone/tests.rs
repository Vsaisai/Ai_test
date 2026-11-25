//! Tests for the F95Zone provider: the feed JSON, both HTML parsers and robustness.

use super::*;
    use super::*;

    const LIST: &str = r#"{"status":"ok","msg":{"data":[
      {"thread_id":319968,"title":"La Previa","creator":"rioplatagames","version":"v1.3","views":18,"likes":0,
       "prefixes":[13,7,18],"tags":[130,173,237],"rating":0,"cover":"https:\/\/preview.f95zone.to\/2026\/11\/x.png",
       "screens":[],"date":"2 hrs","watched":false,"ignored":false,"new":true,"ts":1794222906},
      {"thread_id":"1000","title":"Old &amp; Gold","creator":"Dev","version":"Unknown","views":"5","likes":2,
       "prefixes":[],"tags":[75],"rating":4.5,"cover":"","screens":[],"ts":0}
    ],"pagination":{"page":1,"total":210},"count":18900}}"#;

    #[test]
    fn list_feed() {
        let mut tags = fallback_tags();
        tags.insert(130, "3dcg".into());
        let prefixes: HashMap<u64, String> = [
            (7, "Ren'Py".to_string()),
            (18, "Completed".to_string()),
            (13, "VN".to_string()),
        ]
        .into_iter()
        .collect();
        let p = parse_list(LIST, &tags, &prefixes).unwrap();
        assert_eq!(p.total_pages, Some(210));
        let g = &p.games[0];
        assert_eq!(g.ext_id, "319968");
        assert_eq!(g.title, "La Previa");
        assert_eq!(g.developer, "rioplatagames");
        assert_eq!(
            (g.status.as_str(), g.engine.as_str()),
            ("completed", "Ren'Py")
        );
        assert!(g.tags.contains(&"3dcg".to_string()));
        assert_eq!(g.updated.as_deref(), Some("2026-11-09"));
        assert_eq!(g.page_url, "https://f95zone.to/threads/319968/");
        let o = &p.games[1];
        assert_eq!(o.title, "Old & Gold");
        assert_eq!(o.version, "");
        assert_eq!(o.views, 5);
        assert_eq!(o.cover, None);
        assert_eq!(o.updated.as_deref(), Some("1970-01-01"));
        // the snapshot names tag 75, so the content policy can see it
        assert!(super::super::is_forbidden(&o.tags));
    }

    #[test]
    fn errors_stop_the_import() {
        let blocked = r#"{"status":"error","msg":"You have been temporarily blocked because of a large amount of requests, please try again later"}"#;
        assert!(matches!(
            parse_list(blocked, &HashMap::new(), &HashMap::new()),
            Err(ProviderError::RateLimited { .. })
        ));
        assert!(matches!(
            check_response(200, "<html><title>Just a moment...</title>"),
            Err(ProviderError::Blocked(_))
        ));
        assert!(matches!(
            check_response(429, ""),
            Err(ProviderError::RateLimited { .. })
        ));
        assert!(matches!(
            check_response(403, ""),
            Err(ProviderError::Blocked(_))
        ));
        assert!(matches!(
            check_response(502, ""),
            Err(ProviderError::Unavailable(_))
        ));
        assert!(check_response(200, "{}").is_ok());
    }

    #[test]
    fn dictionaries_from_the_page() {
        let page = r#"<script>var latestUpdates = {"tags":{"1":"2d game","130":"3dcg"},
          "options":{"prefixes":{"games":[{"id":1,"name":"Engine","prefixes":[{"id":7,"name":"Ren'Py"},{"id":18,"name":"Completed"}]}]}}};
          var other = 1;</script>"#;
        let (t, p) = parse_dictionaries(page).unwrap();
        assert_eq!(t.get(&130).map(String::as_str), Some("3dcg"));
        assert_eq!(p.get(&7).map(String::as_str), Some("Ren'Py"));
        assert_eq!(p.get(&18).map(String::as_str), Some("Completed"));
        assert!(parse_dictionaries("<html>login</html>").is_err());
    }

    const THREAD: &str = r#"<html><head><title>Summer Nights [v0.5] [Acme] | F95zone</title></head><body>
      <h1 class="p-title-value"><a href="/forums/games.2/?prefix_id=7" class="labelLink"><span class="label label--gray" dir="auto">Ren'Py</span></a><span class="label-append">&nbsp;</span><span class="label label--blue" dir="auto">Completed</span><span class="label-append">&nbsp;</span>Summer Nights [v0.5] [Acme]</h1>
      <dd><span class="js-tagList"><a href="/tags/3dcg/" class="tagItem">3dcg</a><a href="/tags/male-protagonist/" class="tagItem">male protagonist</a></span></dd>
      <article class="message"><div class="bbWrapper"><div style="text-align: center"><img src="https://attachments.f95zone.to/2026/11/cover.png" data-src="https://attachments.f95zone.to/2026/11/cover.png" /></div>
      <b>Overview:</b><br />A quiet town &amp; one long summer.<br />Make choices that matter.<br /><br />
      <b>Thread Updated</b>: 2026-10-30<br /><b>Release Date</b>: 2026-10-29<br />
      <b>Developer</b>: Acme - <a href="https://www.patreon.com/acme" target="_blank">Patreon</a> - <a href="https://acme.itch.io/summer-nights">Itch.io</a> - <a href="https://f95zone.to/masked/xyz">Site</a><br />
      <b>Censored</b>: No<br /><b>Version</b>: 0.5<br />
      <b>DOWNLOAD</b><br /><a href="https://mega.nz/file/abc">MEGA</a> - <a href="https://www.patreon.com/posts/build-123">Patreon build</a>
      </div></article></body></html>"#;

    #[test]
    fn game_page() {
        let g = parse_thread(THREAD, "4242").unwrap();
        assert_eq!(g.title, "Summer Nights");
        assert_eq!(g.developer, "Acme");
        assert_eq!(g.version, "0.5");
        assert_eq!(
            (g.status.as_str(), g.engine.as_str()),
            ("completed", "Ren'Py")
        );
        assert_eq!(g.tags, vec!["3dcg", "male protagonist"]);
        assert_eq!(g.updated.as_deref(), Some("2026-10-30"));
        assert_eq!(
            g.description.as_deref(),
            Some("A quiet town & one long summer.\nMake choices that matter.")
        );
        let urls: Vec<&str> = g.links.iter().map(|l| l.url.as_str()).collect();
        // only official pages above the download section; no mirrors, no forum redirects
        assert_eq!(
            urls,
            vec![
                "https://www.patreon.com/acme",
                "https://acme.itch.io/summer-nights"
            ]
        );
        assert_eq!(g.links[1].label, "Itch.io");
        assert_eq!(
            g.cover.as_deref(),
            Some("https://attachments.f95zone.to/2026/11/cover.png")
        );
        assert!(matches!(
            parse_thread("<html><title>Log in | F95zone</title></html>", "1"),
            Err(ProviderError::Blocked(_))
        ));
    }

    const FORUM: &str = r#"<html><body>
      <div class="structItemContainer-group structItemContainer-group--sticky">
        <div class="structItem structItem--thread js-inlineModContainer js-threadListItem-5"><div class="structItem-title">
          <a href="/threads/f95zone-games-rules.5/">F95Zone Games Rules</a></div></div>
      </div>
      <div class="structItemContainer-group js-threadList">
        <div class="structItem structItem--thread js-inlineModContainer js-threadListItem-319968"><div class="structItem-title">
          <a href="/forums/games.2/?prefix_id=7" class="labelLink" rel="nofollow"><span class="label label--gray" dir="auto">Ren'Py</span></a>
          <a href="/threads/la-previa.319968/" data-tp-primary="on">La Previa [v1.3] [rioplatagames]</a></div></div>
        <div class="structItem structItem--thread js-inlineModContainer js-threadListItem-1000"><div class="structItem-title">
          <a href="/forums/games.2/?prefix_id=18" class="labelLink"><span class="label label--blue">Completed</span></a>
          <a href="/forums/games.2/?prefix_id=3" class="labelLink"><span class="label label--gray">Unity</span></a>
          <a href="/threads/old-gold.1000/">Old &amp; Gold [Final] [Dev Team]</a></div></div>
        <div class="structItem structItem--thread js-inlineModContainer js-threadListItem-1001"><div class="structItem-title">
          <a href="/threads/no-brackets.1001/">No Brackets</a></div></div>
      </div>
      <ul class="pageNav-main"><li><a href="/forums/games.2/page-2">2</a></li><li><a href="/forums/games.2/page-987">987</a></li></ul>
    </body></html>"#;

    #[test]
    fn forum_list() {
        let p = parse_forum_page(FORUM).unwrap();
        assert_eq!(p.last_page, Some(987));
        assert_eq!(p.service, 1);
        let ids: Vec<&str> = p.games.iter().map(|g| g.ext_id.as_str()).collect();
        assert_eq!(ids, vec!["319968", "1000", "1001"]);
        let g = &p.games[0];
        assert_eq!(
            (g.title.as_str(), g.version.as_str(), g.developer.as_str()),
            ("La Previa", "v1.3", "rioplatagames")
        );
        assert_eq!(
            (g.status.as_str(), g.engine.as_str()),
            ("ongoing", "Ren'Py")
        );
        let o = &p.games[1];
        assert_eq!(
            (
                o.title.as_str(),
                o.developer.as_str(),
                o.status.as_str(),
                o.engine.as_str()
            ),
            ("Old & Gold", "Dev Team", "completed", "Unity")
        );
        assert_eq!(
            (p.games[2].title.as_str(), p.games[2].developer.as_str()),
            ("No Brackets", "")
        );
        assert!(matches!(
            parse_forum_page("<html><title>Just a moment...</title></html>"),
            Err(ProviderError::Blocked(_))
        ));
        assert!(parse_forum_page("<html>nothing</html>").is_err());
    }

    #[test]
    fn whole_description_is_kept() {
        let long: String = (0..400)
            .map(|i| format!("Sentence number {i} of the story. "))
            .collect();
        let page = THREAD.replace(
            "A quiet town &amp; one long summer.<br />Make choices that matter.",
            &long,
        );
        let g = parse_thread(&page, "1").unwrap();
        let d = g.description.unwrap();
        assert!(d.chars().count() > 10_000, "{}", d.len());
        assert!(d.ends_with("story."));
    }

    /// A page shaped like the real ones: a notice with its own text block above the thread,
    /// prefixes as links, the first post marked as the thread starter, values on the next line.
    const THREAD2: &str = r#"<html><head><title>VN - Ren'Py - Winter Tale [Ch. 3] [Northern Lights Studio] | F95zone</title></head>
      <body data-template="thread_view">
      <div class="notice-content"><div class="bbWrapper">Site notice: <a href="https://www.patreon.com/notthegame">support us</a></div></div>
      <div class="p-body-header"><h1 class="p-title-value"><a href="/forums/games.2/?prefix_id=13" class="labelLink" rel="nofollow"><span class="label label--red" dir="auto">VN</span></a><span class="label-append">&nbsp;</span><a href="/forums/games.2/?prefix_id=7" class="labelLink" rel="nofollow"><span class="label label--lightGreen" dir="auto">Ren&#039;Py</span></a><span class="label-append">&nbsp;</span><a href="/forums/games.2/?prefix_id=22" class="labelLink" rel="nofollow"><span class="label label--orange" dir="auto">Onhold</span></a><span class="label-append">&nbsp;</span>Winter Tale [Ch. 3] [Northern Lights Studio]</h1>
      <dl class="tagList"><dt>Tags</dt><dd><span class="js-tagList"><a href="/tags/3dcg/" class="tagItem" dir="auto">3dcg</a> <a href="/tags/asset-addon/" class="tagItem" dir="auto">asset-addon</a> <a href="/tags/male-protagonist/" class="tagItem" dir="auto">male protagonist</a></span></dd></dl></div>
      <article class="message message--post message-threadStarterPost js-post" data-author="uploader"><div class="message-inner">
      <article class="message-body js-selectToQuote"><div class="bbWrapper"><div class="lbContainer lbContainer--inline" data-src="https://attachments.f95zone.to/2026/11/12345_cover.png"><img src="https://attachments.f95zone.to/2026/11/12345_cover.png" class="bbImage" /></div><br />
      <b>Overview:</b><br />
      A long winter in a small town – and a letter that changes everything.<br />
      <br />
      <b>Thread Updated</b>: 2026/10/30<br />
      <b>Release Date</b>: 2026-10-29<br />
      <b>Developer</b>:<br />
      Northern Lights Studio <a href="https://www.patreon.com/northernlights" class="link link--external">Patreon</a> - <a href="https://northernlights.itch.io/winter-tale" class="link link--external">Itch.io</a> - <a href="https://discord.gg/abc" class="link link--external">Discord</a><br />
      <b>Censored</b>: No<br />
      <b>Game Version</b>: Ch. 3<br />
      <b>OS</b>: Windows, Linux<br />
      <b>Genre</b>:<br />
      <div class="bbCodeSpoiler"><button class="bbCodeSpoiler-button"><span>Spoiler</span></button><div class="bbCodeSpoiler-content">3DCG, Male protagonist</div></div>
      <div style="text-align: center"><span style="font-size: 18px"><b>DOWNLOAD</b></span><br />
      <b>Win/Linux</b>: <a href="https://f95zone.to/masked/mega.nz/1/2/abc" class="link link--external">MEGA</a> - <a href="https://pixeldrain.com/u/x">PIXELDRAIN</a></div>
      </div></article></div></article>
      <article class="message message--post js-post"><article class="message-body"><div class="bbWrapper">A reply with <a href="https://www.patreon.com/someoneelse">another link</a></div></article></article>
      </body></html>"#;

    #[test]
    fn game_page_shaped_like_the_real_ones() {
        let g = parse_thread(THREAD2, "777").unwrap();
        assert_eq!(g.title, "Winter Tale");
        assert_eq!(g.developer, "Northern Lights Studio");
        assert_eq!(g.version, "Ch. 3");
        assert_eq!((g.status.as_str(), g.engine.as_str()), ("onhold", "Ren'Py"));
        // names as the feed gives them: the hyphen of "asset-addon" stays
        assert_eq!(g.tags, vec!["3dcg", "asset-addon", "male protagonist"]);
        assert_eq!(g.updated.as_deref(), Some("2026-10-30"));
        assert_eq!(
            g.description.as_deref(),
            Some("A long winter in a small town – and a letter that changes everything.")
        );
        assert_eq!(
            g.cover.as_deref(),
            Some("https://attachments.f95zone.to/2026/11/12345_cover.png")
        );
        // only the first post, only above DOWNLOAD: not the notice, not the reply, no file hosts
        let urls: Vec<&str> = g.links.iter().map(|l| l.url.as_str()).collect();
        assert_eq!(
            urls,
            vec![
                "https://www.patreon.com/northernlights",
                "https://northernlights.itch.io/winter-tale",
                "https://discord.gg/abc"
            ]
        );
    }

    #[test]
    fn developer_names_lose_their_link_labels() {
        assert_eq!(
            clean_developer("Acme Games - Patreon - Discord"),
            "Acme Games"
        );
        assert_eq!(clean_developer("Acme Games Patreon Discord"), "Acme Games");
        assert_eq!(clean_developer("Acme – Website | Itch.io"), "Acme");
        assert_eq!(clean_developer("Dr. X (formerly Y)"), "Dr. X");
        assert_eq!(clean_developer("Patreon"), "Patreon");
        // no name on the line at all: nothing, the title's "[Developer]" is used instead
        assert_eq!(clean_developer(" - Patreon"), "");
        // fields: value on the next line, colon on the next line, longer names first
        assert_eq!(
            field("Developer:\n\nAcme", &["Developer"]).as_deref(),
            Some("Acme")
        );
        assert_eq!(
            field("Developer\n: Acme", &["Developer"]).as_deref(),
            Some("Acme")
        );
        assert_eq!(
            field("Developers: A & B\nDeveloper: C", DEVELOPER_FIELDS).as_deref(),
            Some("A & B")
        );
        assert_eq!(field("Developer notes are here", &["Developer"]), None);
    }

    #[test]
    fn multibyte_text_never_breaks_parsing() {
        // the exact crash from a real page: '&' then an en dash right at byte 12
        assert_eq!(html_unescape("& 123456789–x"), "& 123456789–x");
        assert_eq!(html_unescape("&ampx–y; z"), "&ampx–y; z");
        assert_eq!(field("Release Dat–e: 2020", &["Release Date"]), None);
        assert_eq!(
            field("Release Date: 2025–11-01", &["Release Date"]).as_deref(),
            Some("2025–11-01")
        );
        // characters of 2, 3 and 4 bytes, and one whose lower case is longer than itself
        for ch in ["–", "İ", "😀", "é"] {
            for (name, page) in [("thread", THREAD), ("thread2", THREAD2), ("forum", FORUM)] {
                let mut at = 0;
                while at <= page.len() {
                    if page.is_char_boundary(at) {
                        let mut t = String::with_capacity(page.len() + 4);
                        t.push_str(&page[..at]);
                        t.push_str(ch);
                        t.push_str(&page[at..]);
                        let r = std::panic::catch_unwind(|| {
                            let _ = parse_thread(&t, "1");
                            let _ = parse_forum_page(&t);
                            let _ = html_text(&t);
                            let _ = html_unescape(&t);
                        });
                        assert!(r.is_ok(), "{name}: {ch:?} at byte {at}");
                    }
                    at += 1;
                }
            }
        }
    }

    #[test]
    fn dates_and_entities() {
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(951_782_400), "2000-02-29");
        assert_eq!(
            html_unescape("a &amp; b &#39;c&#x27; &bogus d"),
            "a & b 'c' &bogus d"
        );
        assert_eq!(excerpt("One. Two. Three.", 10), "One. Two.…");
        // a line break in the source is a space; <br> is a line; <br><br> a paragraph
        assert_eq!(
            html_text("one<br />\ntwo\nstill two<br />\n<br />\nthree"),
            "one\ntwo still two\n\nthree"
        );
    }

