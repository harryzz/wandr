//! In-memory protocol store for the task-67 link + receive flow. `Rc<Inner>` so
//! `Clone` yields a shared handle (the `ServiceCipher` clones the store but must
//! see the same sessions). Single-threaded guest ⇒ `RefCell`/`Cell` + `?Send`
//! async-trait. Identity is settable post-link (provisioning reveals it). Not
//! persisted (v1).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use libsignal_service::pre_keys::{KyberPreKeyStoreExt, PreKeysStore};
// serialize/deserialize for SignedPreKeyRecord + KyberPreKeyRecord come from this.
use libsignal_service::protocol::GenericSignedPreKey;
use libsignal_service::protocol::{
    DeviceId, Direction, IdentityChange, IdentityKey, IdentityKeyPair,
    IdentityKeyStore, KyberPreKeyId, KyberPreKeyRecord, KyberPreKeyStore,
    PreKeyId, PreKeyRecord, PreKeyStore, ProtocolAddress, ProtocolStore,
    PublicKey, SenderKeyRecord, SenderKeyStore, ServiceId, SessionRecord,
    SessionStore, SignalProtocolError, SignedPreKeyId, SignedPreKeyRecord,
    SignedPreKeyStore,
};
use libsignal_service::session_store::SessionStoreExt;

struct Inner {
    identity: RefCell<IdentityKeyPair>,
    registration_id: Cell<u32>,
    pre_keys: RefCell<HashMap<u32, PreKeyRecord>>,
    signed: RefCell<HashMap<u32, SignedPreKeyRecord>>,
    kyber: RefCell<HashMap<u32, KyberPreKeyRecord>>,
    last_resort: RefCell<HashSet<u32>>,
    identities: RefCell<HashMap<String, IdentityKey>>,
    sessions: RefCell<HashMap<String, SessionRecord>>,
    sender_keys: RefCell<HashMap<String, SenderKeyRecord>>,
    next_signed: Cell<u32>,
    next_pq: Cell<u32>,
    next_prekey: Cell<u32>,
    last_signed: Cell<Option<u32>>,
    last_kyber: Cell<Option<u32>>,
}

#[derive(Clone)]
pub struct MemStore {
    inner: Rc<Inner>,
}

impl MemStore {
    pub fn new(identity: IdentityKeyPair, registration_id: u32) -> Self {
        MemStore {
            inner: Rc::new(Inner {
                identity: RefCell::new(identity),
                registration_id: Cell::new(registration_id),
                pre_keys: RefCell::new(HashMap::new()),
                signed: RefCell::new(HashMap::new()),
                kyber: RefCell::new(HashMap::new()),
                last_resort: RefCell::new(HashSet::new()),
                identities: RefCell::new(HashMap::new()),
                sessions: RefCell::new(HashMap::new()),
                sender_keys: RefCell::new(HashMap::new()),
                next_signed: Cell::new(0),
                next_pq: Cell::new(0),
                next_prekey: Cell::new(0),
                last_signed: Cell::new(None),
                last_kyber: Cell::new(None),
            }),
        }
    }

    /// Replace the identity + registration id once provisioning reveals them.
    pub fn set_identity(&self, identity: IdentityKeyPair, registration_id: u32) {
        *self.inner.identity.borrow_mut() = identity;
        self.inner.registration_id.set(registration_id);
    }

    pub fn identity(&self) -> IdentityKeyPair {
        *self.inner.identity.borrow()
    }

    /// Any stored identity for a protocol-address `name` (an ACI's service-id
    /// string), scanning across devices — the identity key is account-level, so
    /// device 1's exact entry may be absent even when we hold the peer's key.
    pub fn identity_for_name(&self, name: &str) -> Option<IdentityKey> {
        let prefix = format!("{name}.");
        self.inner
            .identities
            .borrow()
            .iter()
            .find(|(k, _)| k.starts_with(&prefix))
            .map(|(_, v)| *v)
    }
}

// SAFETY: the guest is single-threaded (wstd, no thread::spawn), so the `Rc` +
// `RefCell` interior is never actually shared/sent across threads. `MessageSender`
// requires `S: Sync`; assert it for our single-threaded use.
unsafe impl Sync for MemStore {}
unsafe impl Send for MemStore {}

fn addr_key(address: &ProtocolAddress) -> String {
    format!("{}.{}", address.name(), u32::from(address.device_id()))
}

#[async_trait(?Send)]
impl PreKeyStore for MemStore {
    async fn get_pre_key(
        &self,
        id: PreKeyId,
    ) -> Result<PreKeyRecord, SignalProtocolError> {
        self.inner
            .pre_keys
            .borrow()
            .get(&u32::from(id))
            .cloned()
            .ok_or(SignalProtocolError::InvalidPreKeyId)
    }

    async fn save_pre_key(
        &mut self,
        id: PreKeyId,
        record: &PreKeyRecord,
    ) -> Result<(), SignalProtocolError> {
        let k = u32::from(id);
        self.inner.pre_keys.borrow_mut().insert(k, record.clone());
        self.inner.next_prekey.set(k + 1);
        Ok(())
    }

    async fn remove_pre_key(
        &mut self,
        id: PreKeyId,
    ) -> Result<(), SignalProtocolError> {
        self.inner.pre_keys.borrow_mut().remove(&u32::from(id));
        Ok(())
    }
}

#[async_trait(?Send)]
impl SignedPreKeyStore for MemStore {
    async fn get_signed_pre_key(
        &self,
        id: SignedPreKeyId,
    ) -> Result<SignedPreKeyRecord, SignalProtocolError> {
        self.inner
            .signed
            .borrow()
            .get(&u32::from(id))
            .cloned()
            .ok_or(SignalProtocolError::InvalidSignedPreKeyId)
    }

    async fn save_signed_pre_key(
        &mut self,
        id: SignedPreKeyId,
        record: &SignedPreKeyRecord,
    ) -> Result<(), SignalProtocolError> {
        let k = u32::from(id);
        self.inner.signed.borrow_mut().insert(k, record.clone());
        self.inner.last_signed.set(Some(k));
        self.inner.next_signed.set(k + 1);
        Ok(())
    }
}

#[async_trait(?Send)]
impl KyberPreKeyStore for MemStore {
    async fn get_kyber_pre_key(
        &self,
        id: KyberPreKeyId,
    ) -> Result<KyberPreKeyRecord, SignalProtocolError> {
        self.inner
            .kyber
            .borrow()
            .get(&u32::from(id))
            .cloned()
            .ok_or(SignalProtocolError::InvalidKyberPreKeyId)
    }

    async fn save_kyber_pre_key(
        &mut self,
        id: KyberPreKeyId,
        record: &KyberPreKeyRecord,
    ) -> Result<(), SignalProtocolError> {
        let k = u32::from(id);
        self.inner.kyber.borrow_mut().insert(k, record.clone());
        self.inner.last_kyber.set(Some(k));
        self.inner.next_pq.set(k + 1);
        Ok(())
    }

    async fn mark_kyber_pre_key_used(
        &mut self,
        _id: KyberPreKeyId,
        _signed_pre_key_id: SignedPreKeyId,
        _base_key: &PublicKey,
    ) -> Result<(), SignalProtocolError> {
        Ok(())
    }
}

#[async_trait(?Send)]
impl KyberPreKeyStoreExt for MemStore {
    async fn store_last_resort_kyber_pre_key(
        &mut self,
        id: KyberPreKeyId,
        record: &KyberPreKeyRecord,
    ) -> Result<(), SignalProtocolError> {
        let k = u32::from(id);
        self.inner.kyber.borrow_mut().insert(k, record.clone());
        self.inner.last_resort.borrow_mut().insert(k);
        self.inner.last_kyber.set(Some(k));
        self.inner.next_pq.set(k + 1);
        Ok(())
    }

    async fn load_last_resort_kyber_pre_keys(
        &self,
    ) -> Result<Vec<KyberPreKeyRecord>, SignalProtocolError> {
        let kyber = self.inner.kyber.borrow();
        Ok(self
            .inner
            .last_resort
            .borrow()
            .iter()
            .filter_map(|k| kyber.get(k).cloned())
            .collect())
    }

    async fn remove_kyber_pre_key(
        &mut self,
        id: KyberPreKeyId,
    ) -> Result<(), SignalProtocolError> {
        let k = u32::from(id);
        self.inner.kyber.borrow_mut().remove(&k);
        self.inner.last_resort.borrow_mut().remove(&k);
        Ok(())
    }

    async fn mark_all_one_time_kyber_pre_keys_stale_if_necessary(
        &mut self,
        _stale_time: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), SignalProtocolError> {
        Ok(())
    }

    async fn delete_all_stale_one_time_kyber_pre_keys(
        &mut self,
        _threshold: chrono::DateTime<chrono::Utc>,
        _min_count: usize,
    ) -> Result<(), SignalProtocolError> {
        Ok(())
    }
}

#[async_trait(?Send)]
impl IdentityKeyStore for MemStore {
    async fn get_identity_key_pair(
        &self,
    ) -> Result<IdentityKeyPair, SignalProtocolError> {
        Ok(*self.inner.identity.borrow())
    }

    async fn get_local_registration_id(
        &self,
    ) -> Result<u32, SignalProtocolError> {
        Ok(self.inner.registration_id.get())
    }

    async fn save_identity(
        &mut self,
        address: &ProtocolAddress,
        identity: &IdentityKey,
    ) -> Result<IdentityChange, SignalProtocolError> {
        let prev = self
            .inner
            .identities
            .borrow_mut()
            .insert(addr_key(address), *identity);
        Ok(match prev {
            Some(p) if p != *identity => IdentityChange::ReplacedExisting,
            _ => IdentityChange::NewOrUnchanged,
        })
    }

    async fn is_trusted_identity(
        &self,
        _address: &ProtocolAddress,
        _identity: &IdentityKey,
        _direction: Direction,
    ) -> Result<bool, SignalProtocolError> {
        Ok(true) // trust on first use (v1)
    }

    async fn get_identity(
        &self,
        address: &ProtocolAddress,
    ) -> Result<Option<IdentityKey>, SignalProtocolError> {
        Ok(self.inner.identities.borrow().get(&addr_key(address)).copied())
    }
}

#[async_trait(?Send)]
impl SessionStore for MemStore {
    async fn load_session(
        &self,
        address: &ProtocolAddress,
    ) -> Result<Option<SessionRecord>, SignalProtocolError> {
        Ok(self.inner.sessions.borrow().get(&addr_key(address)).cloned())
    }

    async fn store_session(
        &mut self,
        address: &ProtocolAddress,
        record: &SessionRecord,
    ) -> Result<(), SignalProtocolError> {
        self.inner
            .sessions
            .borrow_mut()
            .insert(addr_key(address), record.clone());
        Ok(())
    }
}

#[async_trait(?Send)]
impl SenderKeyStore for MemStore {
    async fn store_sender_key(
        &mut self,
        sender: &ProtocolAddress,
        distribution_id: Uuid,
        record: &SenderKeyRecord,
    ) -> Result<(), SignalProtocolError> {
        let key = format!("{}|{}", addr_key(sender), distribution_id);
        self.inner.sender_keys.borrow_mut().insert(key, record.clone());
        Ok(())
    }

    async fn load_sender_key(
        &mut self,
        sender: &ProtocolAddress,
        distribution_id: Uuid,
    ) -> Result<Option<SenderKeyRecord>, SignalProtocolError> {
        let key = format!("{}|{}", addr_key(sender), distribution_id);
        Ok(self.inner.sender_keys.borrow().get(&key).cloned())
    }
}

impl ProtocolStore for MemStore {}

#[async_trait(?Send)]
impl SessionStoreExt for MemStore {
    async fn get_sub_device_sessions(
        &self,
        name: &ServiceId,
    ) -> Result<Vec<DeviceId>, SignalProtocolError> {
        // Sessions key on `service_id_string().device` (= to_protocol_address).
        // Return every device *except* the primary (1) we have a session with —
        // required for multi-device SEND fan-out.
        let prefix = format!("{}.", name.service_id_string());
        let mut out = Vec::new();
        for key in self.inner.sessions.borrow().keys() {
            if let Some(dev) = key.strip_prefix(&prefix) {
                if let Ok(d) = dev.parse::<u32>() {
                    if d != 1 {
                        if let Ok(id) = DeviceId::try_from(d) {
                            out.push(id);
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    async fn delete_session(
        &self,
        address: &ProtocolAddress,
    ) -> Result<(), SignalProtocolError> {
        self.inner.sessions.borrow_mut().remove(&addr_key(address));
        Ok(())
    }

    async fn delete_all_sessions(
        &self,
        address: &ServiceId,
    ) -> Result<usize, SignalProtocolError> {
        let prefix = format!("{}.", address.service_id_string());
        let mut sessions = self.inner.sessions.borrow_mut();
        let before = sessions.len();
        sessions.retain(|k, _| !k.starts_with(&prefix));
        Ok(before - sessions.len())
    }
}

#[async_trait(?Send)]
impl PreKeysStore for MemStore {
    async fn next_pre_key_id(&self) -> Result<u32, SignalProtocolError> {
        Ok(self.inner.next_prekey.get())
    }

    async fn next_signed_pre_key_id(&self) -> Result<u32, SignalProtocolError> {
        Ok(self.inner.next_signed.get())
    }

    async fn next_pq_pre_key_id(&self) -> Result<u32, SignalProtocolError> {
        Ok(self.inner.next_pq.get())
    }

    async fn signed_pre_keys_count(
        &self,
    ) -> Result<usize, SignalProtocolError> {
        Ok(self.inner.signed.borrow().len())
    }

    async fn kyber_pre_keys_count(
        &self,
        last_resort: bool,
    ) -> Result<usize, SignalProtocolError> {
        let total = self.inner.kyber.borrow().len();
        let lr = self.inner.last_resort.borrow().len();
        Ok(if last_resort { lr } else { total - lr })
    }

    async fn signed_prekey_id(
        &self,
    ) -> Result<Option<SignedPreKeyId>, SignalProtocolError> {
        Ok(self.inner.last_signed.get().map(Into::into))
    }

    async fn last_resort_kyber_prekey_id(
        &self,
    ) -> Result<Option<KyberPreKeyId>, SignalProtocolError> {
        Ok(self.inner.last_kyber.get().map(Into::into))
    }
}

// ---- persistence: serialize the protocol maps to a JSON snapshot ----

#[derive(Serialize, Deserialize, Default)]
struct Snapshot {
    pre_keys: HashMap<u32, Vec<u8>>,
    signed: HashMap<u32, Vec<u8>>,
    kyber: HashMap<u32, Vec<u8>>,
    last_resort: Vec<u32>,
    identities: HashMap<String, Vec<u8>>,
    sessions: HashMap<String, Vec<u8>>,
    sender_keys: HashMap<String, Vec<u8>>,
    next_signed: u32,
    next_pq: u32,
    next_prekey: u32,
    last_signed: Option<u32>,
    last_kyber: Option<u32>,
}

impl MemStore {
    /// Snapshot the protocol maps (not the identity/regid — those live in the
    /// persisted Account and are restored via `set_identity`).
    pub fn snapshot_bytes(&self) -> Vec<u8> {
        let i = &self.inner;
        let snap = Snapshot {
            pre_keys: i.pre_keys.borrow().iter()
                .map(|(k, v)| (*k, v.serialize().expect("ser prekey"))).collect(),
            signed: i.signed.borrow().iter()
                .map(|(k, v)| (*k, v.serialize().expect("ser signed"))).collect(),
            kyber: i.kyber.borrow().iter()
                .map(|(k, v)| (*k, v.serialize().expect("ser kyber"))).collect(),
            last_resort: i.last_resort.borrow().iter().copied().collect(),
            identities: i.identities.borrow().iter()
                .map(|(k, v)| (k.clone(), v.serialize().to_vec())).collect(),
            sessions: i.sessions.borrow().iter()
                .map(|(k, v)| (k.clone(), v.serialize().expect("ser session"))).collect(),
            sender_keys: i.sender_keys.borrow().iter()
                .map(|(k, v)| (k.clone(), v.serialize().expect("ser sender key"))).collect(),
            next_signed: i.next_signed.get(),
            next_pq: i.next_pq.get(),
            next_prekey: i.next_prekey.get(),
            last_signed: i.last_signed.get(),
            last_kyber: i.last_kyber.get(),
        };
        serde_json::to_vec(&snap).expect("serialize snapshot")
    }

    pub fn load_into(&self, bytes: &[u8]) -> Result<(), String> {
        let snap: Snapshot = serde_json::from_slice(bytes)
            .map_err(|e| format!("snapshot parse: {e}"))?;
        let i = &self.inner;
        let de = |e: SignalProtocolError| e.to_string();

        let mut pk = i.pre_keys.borrow_mut();
        pk.clear();
        for (k, v) in &snap.pre_keys {
            pk.insert(*k, PreKeyRecord::deserialize(v).map_err(de)?);
        }
        drop(pk);

        let mut sp = i.signed.borrow_mut();
        sp.clear();
        for (k, v) in &snap.signed {
            sp.insert(*k, SignedPreKeyRecord::deserialize(v).map_err(de)?);
        }
        drop(sp);

        let mut ky = i.kyber.borrow_mut();
        ky.clear();
        for (k, v) in &snap.kyber {
            ky.insert(*k, KyberPreKeyRecord::deserialize(v).map_err(de)?);
        }
        drop(ky);

        let mut ses = i.sessions.borrow_mut();
        ses.clear();
        for (k, v) in &snap.sessions {
            ses.insert(k.clone(), SessionRecord::deserialize(v).map_err(de)?);
        }
        drop(ses);

        let mut sk = i.sender_keys.borrow_mut();
        sk.clear();
        for (k, v) in &snap.sender_keys {
            sk.insert(k.clone(), SenderKeyRecord::deserialize(v).map_err(de)?);
        }
        drop(sk);

        let mut id = i.identities.borrow_mut();
        id.clear();
        for (k, v) in &snap.identities {
            id.insert(k.clone(), IdentityKey::decode(v).map_err(de)?);
        }
        drop(id);

        *i.last_resort.borrow_mut() = snap.last_resort.iter().copied().collect();
        i.next_signed.set(snap.next_signed);
        i.next_pq.set(snap.next_pq);
        i.next_prekey.set(snap.next_prekey);
        i.last_signed.set(snap.last_signed);
        i.last_kyber.set(snap.last_kyber);
        Ok(())
    }
}
