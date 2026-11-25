use super::*;
use crate::content::{ContentDescriptor, FileKey};
use crate::testing::TestRelay;
use nostr_database::NostrDatabase;

fn shown(name: &str) -> Shown {
    Shown {
        name: name.into(),
        servers: vec!["https://blossom.example".into()],
        file: FileMeta {
            key: FileKey::generate().root().clone(),
            content: ContentDescriptor {
                size: 1,
                chunk_size: 1 << 16,
                chunks: Some(vec![1; 64]),
                table: None,
            },
        },
    }
}

fn live(name: &str) -> impl Fn(&LinkState) -> bool + '_ {
    move |state| matches!(state, LinkState::Live { shown, .. } if shown.name == name)
}

/// The wraps a relay holds for the link's reader key.
async fn posts(relay: &TestRelay, meta: &ShareMeta) -> usize {
    let reader = Keys::new(meta.reader.clone()).public_key();
    let filter = Filter::new().kind(Kind::GiftWrap).pubkey(reader);
    relay.store.count(filter).await.unwrap()
}

#[test]
fn links_round_trip() {
    let link = ShareLink {
        reader: SecretKey::generate(),
        owner: Keys::generate().public_key(),
        relays: vec![
            RelayUrl::parse("wss://relay.ditto.pub").unwrap(),
            RelayUrl::parse("wss://relay.primal.net").unwrap(),
        ],
    };
    let encoded = link.encode();
    assert!(encoded.len() < 200, "{}", encoded.len());
    assert_eq!(ShareLink::decode(&encoded).unwrap(), link);

    let mut tlv = URL_SAFE_NO_PAD.decode(&encoded).unwrap();
    tlv.extend([99, 2, 1, 2]);
    assert_eq!(
        ShareLink::decode(&URL_SAFE_NO_PAD.encode(&tlv)).unwrap(),
        link
    );

    let cut = URL_SAFE_NO_PAD.encode(&tlv[..tlv.len() - 1]);
    assert!(ShareLink::decode(&cut).is_err());
    let without_relays = URL_SAFE_NO_PAD.encode(&tlv[..2 + 32 + 2 + 32]);
    assert!(ShareLink::decode(&without_relays).is_err());
    assert!(ShareLink::decode("not a link!").is_err());
}

#[test]
fn share_keys_stay_in_the_share_node() {
    let meta = ShareMeta {
        reader: SecretKey::generate(),
        owner: SecretKey::generate(),
        relays: vec![RelayUrl::parse("wss://relay.ditto.pub").unwrap()],
        expires: Some(1_900_000_000),
    };
    let json = serde_json::to_value(&meta).unwrap();
    assert_eq!(json["reader"], meta.reader.to_secret_hex());
    assert_eq!(serde_json::from_value::<ShareMeta>(json).unwrap(), meta);
    assert_eq!(
        meta.link().owner,
        Keys::new(meta.owner.clone()).public_key()
    );
}

#[tokio::test]
async fn a_link_shows_its_latest_manifest() {
    let relay = TestRelay::start().await;
    let meta = create(
        std::slice::from_ref(&relay.url),
        &shown("Lupines.jpg"),
        None,
    )
    .await
    .unwrap();
    assert_eq!(meta.relays, std::slice::from_ref(&relay.url));
    assert!(live("Lupines.jpg")(&open(&meta.link()).await.unwrap()));

    publish(&meta, Some(&shown("Lupins.jpg"))).await.unwrap();
    assert!(live("Lupins.jpg")(&open(&meta.link()).await.unwrap()));
    assert_eq!(posts(&relay, &meta).await, 1, "older manifests are deleted");
}

#[tokio::test]
async fn only_the_owner_decides_what_a_link_shows() {
    let relay = TestRelay::start().await;
    let meta = create(
        std::slice::from_ref(&relay.url),
        &shown("Lupines.jpg"),
        None,
    )
    .await
    .unwrap();
    // Someone who has the link knows the reader key, not the owner's.
    let forger = ShareMeta {
        owner: SecretKey::generate(),
        ..meta.clone()
    };
    let forged = wrap(&forger, Some(&shown("Malware.exe"))).unwrap();
    let mailbox = Mailbox::open(&meta.reader, &meta.relays).await.unwrap();
    assert!(!mailbox.relays.send_all(&forged).await.accepted.is_empty());
    mailbox.shutdown().await;

    assert!(live("Lupines.jpg")(&open(&meta.link()).await.unwrap()));
    publish(&meta, Some(&shown("Lupins.jpg"))).await.unwrap();
    assert_eq!(posts(&relay, &meta).await, 1, "the forgery is deleted too");
}

#[tokio::test]
async fn a_stop_is_final() {
    let relay = TestRelay::start().await;
    let meta = create(
        std::slice::from_ref(&relay.url),
        &shown("Lupines.jpg"),
        None,
    )
    .await
    .unwrap();
    publish(&meta, None).await.unwrap();
    assert_eq!(open(&meta.link()).await.unwrap(), LinkState::Revoked);
    assert_eq!(posts(&relay, &meta).await, 1);

    assert!(publish(&meta, Some(&shown("Lupins.jpg"))).await.is_err());
    assert_eq!(open(&meta.link()).await.unwrap(), LinkState::Revoked);
}

#[tokio::test]
async fn expired_links_show_nothing_and_still_stop() {
    let meta = ShareMeta {
        reader: SecretKey::generate(),
        owner: SecretKey::generate(),
        relays: Vec::new(),
        expires: Some(now_secs() - 60),
    };
    let manifest = wrap(&meta, Some(&shown("Lupines.jpg"))).unwrap();
    assert_eq!(
        state(&meta.link(), &[manifest], now_secs()),
        LinkState::Expired
    );

    // Relays refuse what has expired: the stop of an expired link must not.
    let relay = TestRelay::start().await;
    let meta = ShareMeta {
        relays: vec![relay.url.clone()],
        ..meta
    };
    publish(&meta, None).await.unwrap();
    assert_eq!(open(&meta.link()).await.unwrap(), LinkState::Revoked);
}

#[tokio::test]
async fn links_need_a_relay_that_serves_them() {
    let refusing = TestRelay::forgetting(&[Kind::GiftWrap]).await;
    let keeping = TestRelay::start().await;
    let relays = [refusing.url.clone(), keeping.url.clone()];
    let meta = create(&relays, &shown("Lupines.jpg"), None).await.unwrap();
    assert_eq!(meta.relays, std::slice::from_ref(&keeping.url));
    assert!(
        create(&relays[..1], &shown("Lupines.jpg"), None)
            .await
            .is_err()
    );
}
