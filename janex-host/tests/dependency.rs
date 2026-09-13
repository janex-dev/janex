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

/// Lists raw dependency files below the content store.
fn cache_files(path: &std::path::Path) -> Vec<std::path::PathBuf> {
    fn visit(path: &std::path::Path, result: &mut Vec<std::path::PathBuf>) {
        if !path.exists() {
            return;
        }
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, result);
            } else {
                result.push(path);
            }
        }
    }
    let mut result = Vec::new();
    visit(&path.join("files"), &mut result);
    result
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
    assert_eq!(cache.file_name().unwrap(), "library.jar");
    assert_eq!(fs::read(&cache).unwrap(), b"original");
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
        "https://example.com/CON.jar",
        "https://example.com/COM1.jar",
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
fn shared_content_preserves_names_and_repairs_request_metadata() {
    let server = http::Server::new();
    let temp = tempfile::tempdir().unwrap();
    let mut options = DependencyOptions {
        cache_directory: Some(temp.path().into()),
        ..Default::default()
    };
    let pin = digest(b"shared");
    let first = format!("{}/first/library.jar", server.url);
    let second = format!("{}/second/library.jar", server.url);
    server.file("/first/library.jar", b"shared");
    resolve(&first, Some(&pin), &options, false).unwrap();
    resolve(&second, Some(&pin), &options, false).unwrap();
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    assert_eq!(cache_files(temp.path()).len(), 1);
    let records: Vec<_> = fs::read_dir(temp.path().join("metadata/urls"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(records.len(), 2);
    // A record from another request must not be accepted even when its content matches.
    fs::copy(&records[0], &records[1]).unwrap();
    options.offline = true;
    let failures = [&first, &second]
        .iter()
        .filter(|uri| resolve(uri, Some(&pin), &options, false).is_err())
        .count();
    assert_eq!(failures, 1);
    options.offline = false;
    for uri in [&first, &second] {
        resolve(uri, Some(&pin), &options, false).unwrap();
    }
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    server.file("/renamed.jar", b"shared");
    let renamed = resolve(
        &format!("{}/renamed.jar", server.url),
        Some(&pin),
        &options,
        false,
    )
    .unwrap();
    assert_eq!(renamed.jar_name, "renamed.jar");
    assert_eq!(cache_files(temp.path()).len(), 2);
    // Refresh must contact the requested origin despite reusable content.
    options.refresh = true;
    assert!(resolve(&second, Some(&pin), &options, false).is_err());
    options.refresh = false;
    options.offline = true;
    resolve(&second, Some(&pin), &options, false).unwrap();
}

#[test]
fn maven_local_candidates_are_verified_and_never_modified() {
    const CHILD: &str = "JANEX_TEST_MAVEN_CACHE_CHILD";
    if let Ok(root) = std::env::var(CHILD) {
        let options = DependencyOptions {
            cache_directory: Some(std::path::Path::new(&root).join("cache")),
            maven_repository: std::env::var("JANEX_TEST_MAVEN_REPOSITORY").unwrap(),
            ..Default::default()
        };
        let pin = Checksum::compute(Algorithm::Sha512, &b"verified"[..]).unwrap();
        assert_eq!(
            resolve(
                "pkg:maven/org.example/library@1",
                Some(&pin),
                &options,
                false
            )
            .unwrap()
            .bytes,
            b"verified"
        );
        return;
    }
    let server = http::Server::new();
    let temp = tempfile::tempdir().unwrap();
    for (index, candidate) in [b"verified".as_slice(), b"wrong".as_slice()]
        .iter()
        .enumerate()
    {
        let root = temp.path().join(index.to_string());
        let local = root.join(".m2/repository/org/example/library/1/library-1.jar");
        fs::create_dir_all(local.parent().unwrap()).unwrap();
        fs::write(&local, candidate).unwrap();
        server.file("/org/example/library/1/library-1.jar", b"verified");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "maven_local_candidates_are_verified_and_never_modified",
                "--nocapture",
            ])
            .env(CHILD, &root)
            .env("JANEX_TEST_MAVEN_REPOSITORY", &server.url)
            .env("HOME", &root)
            .env("USERPROFILE", &root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read(&local).unwrap(), *candidate);
        assert_eq!(server.requests.lock().unwrap().len(), index);
        assert_eq!(
            fs::read(cache_files(&root.join("cache")).remove(0)).unwrap(),
            b"verified"
        );
    }
}

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
