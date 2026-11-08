//! End-to-end: build a tiny catalog database, load it and run searches.

use rusqlite::{params, Connection};
use vndb_core::search::{EroMode, LangMode, SearchRequest, Sort};
use vndb_core::{search, Catalog};

type Row<'a> = (u32, &'a str, Vec<(&'a str, u8)>, Vec<(u32, f64)>, u32, bool);

fn build(path: &std::path::Path) {
    let c = Connection::open(path).unwrap();
    vndb_core::db::create_schema(&c).unwrap();
    // tags: Theme(1) > Drama(2) > Tragedy(3); Theme > Comedy(4); Style(5) > ADV(6); Sexual Content(7, ero)
    for (id, name, cat) in [
        (1, "Theme", "cont"),
        (2, "Drama", "cont"),
        (3, "Tragedy", "cont"),
        (4, "Comedy", "cont"),
        (5, "Style", "tech"),
        (6, "ADV", "tech"),
        (7, "Sexual Content", "ero"),
    ] {
        c.execute(
            "INSERT INTO tags VALUES (?1, ?2, 0, 1, 1, ?3, '', '')",
            params![id, cat, name],
        )
        .unwrap();
    }
    for (id, parent) in [(2, 1), (3, 2), (4, 1), (6, 5), (7, 1)] {
        c.execute(
            "INSERT INTO tag_parents VALUES (?1, ?2, 1)",
            params![id, parent],
        )
        .unwrap();
    }
    // vn: id, title, langs, tags, votes, ero
    let vns: Vec<Row> = vec![
        (
            1,
            "Zeta",
            vec![("ja", 0), ("en", 1)],
            vec![(3, 2.0), (4, 1.0)],
            100,
            false,
        ),
        (
            2,
            "Alpha",
            vec![("ja", 0), ("en", 0)],
            vec![(4, 1.0)],
            500,
            false,
        ),
        (
            3,
            "Beta",
            vec![("ja", 0), ("ru", 1)],
            vec![(2, 2.0), (4, 1.0), (6, 2.0)],
            50,
            false,
        ),
        (
            4,
            "Aardvark",
            vec![("en", 0)],
            vec![(2, 1.0), (6, 1.0), (7, 2.5)],
            10,
            true,
        ),
        (
            5,
            "Japanese only",
            vec![("ja", 0)],
            vec![(2, 2.0), (4, 2.0), (6, 2.0)],
            1000,
            false,
        ),
        (
            6,
            "MTL only",
            vec![("ja", 0), ("en", 2)],
            vec![(2, 2.0), (4, 2.0), (6, 2.0)],
            5,
            false,
        ),
    ];
    for (id, title, langs, tags, votes, ero) in vns {
        c.execute(
            "INSERT INTO vn VALUES (?1,'ja',NULL,0,0,?2,7.5,7.5,NULL,0,0,0,'','',20200101,?3,-1,0,'win','')",
            params![id, votes, ero],
        )
        .unwrap();
        c.execute(
            "INSERT INTO vn_titles VALUES (?1,'en',1,?2,NULL)",
            params![id, title],
        )
        .unwrap();
        for (l, k) in langs {
            c.execute(
                "INSERT INTO vn_langs(vid, lang, kind) VALUES (?1,?2,?3)",
                params![id, l, k],
            )
            .unwrap();
        }
        for (t, r) in tags {
            c.execute(
                "INSERT INTO vn_tags VALUES (?1,?2,?3,0,0)",
                params![id, t, r],
            )
            .unwrap();
        }
    }
}

fn catalog() -> (tempdir::Dir, Catalog) {
    catalog_with(|_| {})
}

fn catalog_with(tweak: impl Fn(&Connection)) -> (tempdir::Dir, Catalog) {
    let dir = tempdir::Dir::new();
    let p = dir.0.join("c.sqlite");
    build(&p);
    tweak(&Connection::open(&p).unwrap());
    let cat = Catalog::open(&p).unwrap();
    (dir, cat)
}

mod tempdir {
    pub struct Dir(pub std::path::PathBuf);
    impl Dir {
        pub fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "vndb-core-test-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

fn titles(r: &vndb_core::SearchResponse) -> Vec<String> {
    r.items.iter().map(|i| i.title.clone()).collect()
}

#[test]
fn wanted_tags_rank_all_first_then_count_then_alpha() {
    let (_d, cat) = catalog();
    let req = SearchRequest {
        want: vec!["Drama".into(), "Comedy".into(), "ADV".into()],
        ..Default::default()
    };
    let r = search(&cat, &req).unwrap();
    // Drama matches via its sub-tag Tragedy for Zeta. Japanese-only is filtered (readable mode).
    assert_eq!(
        titles(&r),
        vec!["Beta", "MTL only", "Aardvark", "Zeta", "Alpha"]
    );
    assert!(r.items[0].all_wanted);
}

#[test]
fn must_exclude_lang_and_ero() {
    let (_d, cat) = catalog();
    let base = SearchRequest {
        must: vec!["Drama".into()],
        sort: Sort::Title,
        ..Default::default()
    };
    assert_eq!(
        titles(&search(&cat, &base).unwrap()),
        vec!["Aardvark", "Beta", "MTL only", "Zeta"]
    );

    let r = search(
        &cat,
        &SearchRequest {
            exclude: vec!["ADV".into()],
            ..base.clone()
        },
    )
    .unwrap();
    assert_eq!(titles(&r), vec!["Zeta"]);

    let r = search(
        &cat,
        &SearchRequest {
            ero: EroMode::Hide,
            ..base.clone()
        },
    )
    .unwrap();
    assert!(!titles(&r).contains(&"Aardvark".to_string()));

    let r = search(
        &cat,
        &SearchRequest {
            allow_mtl: false,
            ..base.clone()
        },
    )
    .unwrap();
    assert!(!titles(&r).contains(&"MTL only".to_string()));

    let r = search(
        &cat,
        &SearchRequest {
            lang_mode: LangMode::List,
            langs: vec!["ru".into()],
            ..base.clone()
        },
    )
    .unwrap();
    assert_eq!(titles(&r), vec!["Beta"]);

    let r = search(
        &cat,
        &SearchRequest {
            lang_mode: LangMode::Any,
            ..base
        },
    )
    .unwrap();
    assert_eq!(r.total, 5);
}

#[test]
fn text_search_and_tag_counts() {
    let (_d, cat) = catalog();
    let r = search(
        &cat,
        &SearchRequest {
            text: Some("aard".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(titles(&r), vec!["Aardvark"]);
    // Drama count includes VNs tagged only with its child Tragedy
    assert_eq!(cat.tree.get(2).unwrap().vns, 5);
    assert_eq!(
        cat.tree.category(3),
        ("Theme".to_string(), "Drama".to_string())
    );
}

#[test]
fn wanted_tags_do_not_filter_by_default_and_counts_follow_filters() {
    let (_d, cat) = catalog();
    // one wanted tag: VNs without it are still listed, just after the ones that have it
    let r = search(
        &cat,
        &SearchRequest {
            want: vec!["ADV".into()],
            lang_mode: LangMode::Any,
            sort: Sort::Relevance,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.total, 6);
    assert!(r.items[0].all_wanted && !r.items[5].all_wanted);
    // counts are taken over the filtered set (Drama required -> 5 VNs; ADV among them: 4)
    let r = search(
        &cat,
        &SearchRequest {
            must: vec!["Drama".into()],
            lang_mode: LangMode::Any,
            tag_counts: true,
            ..Default::default()
        },
    )
    .unwrap();
    let c = r.tag_counts.unwrap();
    assert_eq!(c.get(&2), Some(&5));
    assert_eq!(c.get(&6), Some(&4));
}

#[test]
fn profile_stats_and_achievements_follow_the_lists() {
    use vndb_core::profile::{compute, Mark};
    let (_d, cat) = catalog();
    let m = |vid, status: &str| Mark {
        vid,
        status: status.into(),
    };
    let empty = compute(&cat, &[], 2026);
    assert_eq!(empty.stats.read, 0);
    assert!(empty.achievements.iter().all(|a| !a.done));

    // two read (one of them favourite), one dropped, one wished; VN 1 has Tragedy (under Drama) and Comedy
    let p = compute(
        &cat,
        &[
            m(1, "finished"),
            m(3, "favorite"),
            m(2, "dropped"),
            m(4, "wish"),
        ],
        2026,
    );
    assert_eq!(
        (
            p.stats.read,
            p.stats.favorite,
            p.stats.dropped,
            p.stats.wish
        ),
        (2, 1, 1, 1)
    );
    let names: Vec<&str> = p.stats.genres.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, vec!["Comedy", "Drama"]);
    let first = p.achievements.iter().find(|a| a.id == "first").unwrap();
    assert!(first.done);
    let five = p.achievements.iter().find(|a| a.id == "reader5").unwrap();
    assert_eq!((five.progress, five.goal, five.done), (2, 5, false));
    // a dropped VN breaks "never give up"
    assert_eq!(
        p.achievements
            .iter()
            .find(|a| a.id == "finisher")
            .unwrap()
            .progress,
        0
    );
    // VN 3 has 50 votes: a hidden gem
    assert!(p.achievements.iter().find(|a| a.id == "gem").unwrap().done);
}

#[test]
fn languages_split_into_ready_releases_and_patches() {
    use vndb_core::search::LangSource;
    // Beta's Russian is a fan patch, not a ready release
    let (_d, cat) = catalog_with(|c| {
        c.execute(
            "UPDATE vn_langs SET ready = 0, patch = 1 WHERE vid = 3 AND lang = 'ru'",
            [],
        )
        .unwrap();
    });
    let ids = |src| {
        let req = SearchRequest {
            lang_mode: LangMode::List,
            langs: vec!["ru".into()],
            lang_source: src,
            ..Default::default()
        };
        let mut v: Vec<String> = search(&cat, &req)
            .unwrap()
            .items
            .into_iter()
            .map(|c| c.id)
            .collect();
        v.sort();
        v
    };
    assert_eq!(ids(LangSource::Any), vec!["v3"]);
    assert!(ids(LangSource::Ready).is_empty());
    assert_eq!(ids(LangSource::Patch), vec!["v3"]);
    let beta = cat.get(3).unwrap();
    assert_eq!(
        (
            beta.ready_langs.contains(&"ru".into()),
            beta.patch_langs.contains(&"ru".into())
        ),
        (false, true)
    );
}

#[test]
fn unknown_way_counts_only_for_any_and_unsplit_catalogs_ignore_the_way() {
    use vndb_core::search::LangSource;
    // Zeta (v1) English: no release says how -> unknown (neither ready nor patch)
    let (_d, mut cat) = catalog_with(|c| {
        c.execute(
            "UPDATE vn_langs SET ready = 0, patch = 0 WHERE vid = 1 AND lang = 'en'",
            [],
        )
        .unwrap();
    });
    let en = |cat: &Catalog, src| {
        let req = SearchRequest {
            lang_mode: LangMode::List,
            langs: vec!["en".into()],
            lang_source: src,
            ..Default::default()
        };
        search(cat, &req)
            .unwrap()
            .items
            .iter()
            .any(|c| c.id == "v1")
    };
    assert!(en(&cat, LangSource::Any));
    assert!(!en(&cat, LangSource::Ready));
    assert!(!en(&cat, LangSource::Patch));
    // an old database that could not be migrated: the way is ignored, nothing disappears
    cat.lang_split = false;
    assert!(en(&cat, LangSource::Ready));
}

#[test]
fn old_catalogs_get_the_language_columns_from_their_releases() {
    let dir = tempdir::Dir::new();
    let p = dir.0.join("old.sqlite");
    build(&p);
    {
        let c = Connection::open(&p).unwrap();
        // releases: a Russian patch for Beta (v3), a full English release for Zeta (v1)
        c.execute_batch(
            "INSERT INTO releases(id, olang, released, minage, has_ero, patch, freeware, uncensored, official, engine) VALUES
               (1, 'ru', 20200101, 0, 0, 1, 1, 0, 0, ''), (2, 'en', 20200101, 0, 0, 0, 0, 0, 1, '');
             INSERT INTO release_titles(rid, lang, mtl, title, latin) VALUES (1, 'ru', 0, 'Beta RU', NULL), (2, 'en', 0, 'Zeta EN', NULL);
             INSERT INTO release_vn(rid, vid, rtype) VALUES (1, 3, 'complete'), (2, 1, 'complete');
             -- a catalog imported by an older version: no ready/patch columns
             CREATE TABLE l2 AS SELECT vid, lang, kind FROM vn_langs; DROP TABLE vn_langs; ALTER TABLE l2 RENAME TO vn_langs;",
        )
        .unwrap();
    }
    let cat = Catalog::open(&p).unwrap();
    assert!(cat.lang_split);
    let beta = cat.get(3).unwrap();
    assert!(beta.patch_langs.contains(&"ru".into()) && !beta.ready_langs.contains(&"ru".into()));
    let zeta = cat.get(1).unwrap();
    assert!(zeta.ready_langs.contains(&"en".into()));
    // Japanese of Zeta has no release in the test data: unknown, neither ready nor patch
    assert!(!zeta.ready_langs.contains(&"ja".into()) && !zeta.patch_langs.contains(&"ja".into()));
}
