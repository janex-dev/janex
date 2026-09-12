// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Network, integrity, cache, and Maven-layout boundaries tested against loopback HTTP.

#[path = "support/http.rs"]
mod http;

use janex_format::checksum::{Algorithm, Checksum};
use janex_host::dependency::{DependencyOptions, resolve};
use std::{fs, time::Duration};

/// Returns a secure checksum for a fixture response.
fn digest(bytes: &[u8]) -> Checksum {
    Checksum::compute(Algorithm::Sha256, bytes).unwrap()
}

/// Lists published cache data, excluding persistent per-entry lock files.
fn cache_files(path: &std::path::Path) -> Vec<std::path::PathBuf> {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "cache")
        })
        .collect()
}

#[test]
fn verifies_downloads_cache_hits_corruption_and_offline_refresh() {
    let server = http::Server::new();
    let temp = tempfile::tempdir().unwrap();
    let mut options = DependencyOptions {
        cache_directory: Some(temp.path().into()),
        ..Default::default()
    };
    let uri = format!("{}/library.jar", server.url);
    let expected = digest(b"original");
    server.file("/library.jar", b"original");
    assert!(resolve(&uri, None, &options, false).is_err());
    assert!(
        resolve(
            &uri,
            Some(&Checksum::compute(Algorithm::Xxh3_64, &b"original"[..]).unwrap()),
            &options,
            true
        )
        .is_err()
    );
    assert!(server.requests.lock().unwrap().is_empty());
    assert_eq!(
        resolve(&uri, Some(&expected), &options, true)
            .unwrap()
            .bytes,
        b"original"
    );
    server.file("/library.jar", b"changed");
    options.offline = true;
    assert_eq!(
        resolve(&uri, Some(&expected), &options, true)
            .unwrap()
            .bytes,
        b"original"
    );
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    let cache = cache_files(temp.path()).remove(0);
    fs::write(&cache, b"corrupt").unwrap();
    assert!(resolve(&uri, Some(&expected), &options, true).is_err());
    options.offline = false;
    assert!(resolve(&uri, Some(&expected), &options, true).is_err());
    assert_eq!(fs::read(&cache).unwrap(), b"corrupt");
    server.file("/library.jar", b"original");
    assert_eq!(
        resolve(&uri, Some(&expected), &options, true)
            .unwrap()
            .bytes,
        b"original"
    );
    let published = fs::read(&cache).unwrap();
    options.refresh = true;
    server.file("/library.jar", b"changed");
    assert!(resolve(&uri, Some(&expected), &options, true).is_err());
    assert_eq!(fs::read(cache).unwrap(), published);
    options.refresh = false;
    options.offline = true;
    options.max_bytes = 2;
    assert!(resolve(&uri, Some(&expected), &options, false).is_err());
}

#[test]
fn bounds_responses_and_redirects_before_cache_publication() {
    let server = http::Server::new();
    let temp = tempfile::tempdir().unwrap();
    let options = DependencyOptions {
        cache_directory: Some(temp.path().into()),
        max_bytes: 4,
        timeout: Duration::from_secs(2),
        ..Default::default()
    };
    let checksum = digest(b"data");
    server.file("/file.jar", b"data");
    server.raw("/redirect.jar", b"HTTP/1.1 302 Found\r\nLocation: /file.jar\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec());
    assert_eq!(
        resolve(
            &format!("{}/redirect.jar", server.url),
            Some(&checksum),
            &options,
            true
        )
        .unwrap()
        .jar_name,
        "redirect.jar"
    );
    server.raw("/loop.jar", b"HTTP/1.1 307 Again\r\nLocation: /loop.jar\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec());
    server.raw("/scheme.jar", b"HTTP/1.1 302 Found\r\nLocation: file:///secret.jar\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec());
    server.raw(
        "/length.jar",
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\ndata".to_vec(),
    );
    server.raw("/chunk.jar", b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nlarge\r\n0\r\n\r\n".to_vec());
    server.raw(
        "/truncated.jar",
        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nx".to_vec(),
    );
    server.raw("/encoding.jar", b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata".to_vec());
    for path in [
        "loop",
        "scheme",
        "length",
        "chunk",
        "truncated",
        "encoding",
        "missing",
    ] {
        assert!(
            resolve(
                &format!("{}/{path}.jar", server.url),
                Some(&checksum),
                &options,
                false
            )
            .is_err(),
            "{path}"
        );
    }
    assert_eq!(cache_files(temp.path()).len(), 1);
}

#[test]
fn maven_coordinates_preserve_names_classifiers_and_repository_identity() {
    let server = http::Server::new();
    let temp = tempfile::tempdir().unwrap();
    let options = DependencyOptions {
        cache_directory: Some(temp.path().into()),
        maven_repository: format!("{}/maven", server.url),
        ..Default::default()
    };
    let checksum = digest(b"jar");
    for (purl, path, filename) in [
        (
            "pkg:maven/org.example/library@1.2.3",
            "/maven/org/example/library/1.2.3/library-1.2.3.jar",
            "library-1.2.3.jar",
        ),
        (
            "pkg:maven/org.example/library@1.2.3?classifier=linux&type=jar",
            "/maven/org/example/library/1.2.3/library-1.2.3-linux.jar",
            "library-1.2.3-linux.jar",
        ),
        (
            "pkg:maven/org.example/library@1.2.3?type=test-jar",
            "/maven/org/example/library/1.2.3/library-1.2.3-tests.jar",
            "library-1.2.3-tests.jar",
        ),
        (
            "pkg:maven/org.example/library@1.2-20260101.123456-7",
            "/maven/org/example/library/1.2-SNAPSHOT/library-1.2-20260101.123456-7.jar",
            "library-1.2-20260101.123456-7.jar",
        ),
    ] {
        server.file(path, b"jar");
        let result = resolve(purl, Some(&checksum), &options, false).unwrap();
        assert_eq!(result.jar_name, filename);
        assert_eq!(server.requests.lock().unwrap().last().unwrap(), path);
    }
    let mut package = packageurl::PackageUrl::from_str("pkg:maven/org.example/library@2").unwrap();
    package
        .add_qualifier("repository_url", format!("{}/other", server.url))
        .unwrap();
    server.file("/other/org/example/library/2/library-2.jar", b"jar");
    resolve(&package.to_string(), Some(&checksum), &options, false).unwrap();
    let requests = server.requests.lock().unwrap().len();
    for uri in [
        "pkg:maven/org.example/library",
        "pkg:maven/org.example/library@1-SNAPSHOT",
        "pkg:maven/org.example/library@1?type=pom",
        "pkg:maven/org.example/library@1?unknown=x",
        "pkg:maven/org.example/library@1#inside",
        "pkg:maven/org.example/library@1?classifier=..",
        "pkg:maven/org.example/library@1?repository_url=file:%2F%2F%2Ftmp",
        "https://example.com/a%2Fb.jar",
        "https://user:password@example.com/file.jar",
    ] {
        assert!(
            resolve(uri, Some(&checksum), &options, false).is_err(),
            "{uri}"
        );
    }
    assert_eq!(server.requests.lock().unwrap().len(), requests);
}

use std::str::FromStr;

#[test]
fn concurrent_publication_never_exposes_partial_cache_entries() {
    let server = http::Server::new();
    let temp = tempfile::tempdir().unwrap();
    let bytes = vec![42; 128 * 1024];
    server.file("/shared.jar", &bytes);
    let checksum = digest(&bytes);
    let options = DependencyOptions {
        cache_directory: Some(temp.path().into()),
        ..Default::default()
    };
    let uri = format!("{}/shared.jar", server.url);
    let barrier = std::sync::Barrier::new(4);
    std::thread::scope(|scope| {
        let mut threads = Vec::new();
        for _ in 0..4 {
            threads.push(scope.spawn(|| {
                barrier.wait();
                assert_eq!(
                    resolve(&uri, Some(&checksum), &options, true)
                        .unwrap()
                        .bytes,
                    bytes
                );
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
    });
    assert_eq!(cache_files(temp.path()).len(), 1);
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    let offline = DependencyOptions {
        offline: true,
        ..options
    };
    assert_eq!(
        resolve(&uri, Some(&checksum), &offline, true)
            .unwrap()
            .bytes,
        bytes
    );
}

#[test]
fn download_deadline_covers_waiting_for_the_response() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let uri = format!("http://{}/slow.jar", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        connection
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let _ = connection.read(&mut [0; 2048]);
        std::thread::sleep(Duration::from_millis(200));
        let _ = connection
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata");
    });
    let temp = tempfile::tempdir().unwrap();
    let options = DependencyOptions {
        cache_directory: Some(temp.path().into()),
        timeout: Duration::from_millis(50),
        ..Default::default()
    };
    assert!(resolve(&uri, Some(&digest(b"data")), &options, false).is_err());
    server.join().unwrap();
    assert!(cache_files(temp.path()).is_empty());
}
