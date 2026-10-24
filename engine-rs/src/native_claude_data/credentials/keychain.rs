//! Apple generic-password attributes and persistent references, never password data.
use anyhow::{bail, Result};
use std::ffi::{c_char, c_void, CString};
type Ref = *const c_void;
const UTF8: u32 = 0x08000100;
const SERVICES: [&str; 3] = [
    "Claude Code",
    "Claude Code-credentials",
    "Claude Code-device-keys",
];

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: Ref);
    fn CFGetTypeID(value: Ref) -> usize;
    fn CFStringCreateWithCString(allocator: Ref, text: *const c_char, encoding: u32) -> Ref;
    fn CFDictionaryCreate(
        allocator: Ref,
        keys: *const Ref,
        values: *const Ref,
        count: isize,
        key_callbacks: Ref,
        value_callbacks: Ref,
    ) -> Ref;
    fn CFDictionaryGetValue(value: Ref, key: Ref) -> Ref;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFArrayCreate(allocator: Ref, values: *const Ref, count: isize, callbacks: Ref) -> Ref;
    fn CFArrayGetCount(value: Ref) -> isize;
    fn CFArrayGetValueAtIndex(value: Ref, index: isize) -> Ref;
    fn CFDataCreate(allocator: Ref, bytes: *const u8, length: isize) -> Ref;
    fn CFDataGetTypeID() -> usize;
    fn CFDataGetLength(value: Ref) -> isize;
    fn CFDataGetBytePtr(value: Ref) -> *const u8;
    fn CFDateGetTypeID() -> usize;
    fn CFDateGetAbsoluteTime(value: Ref) -> f64;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(value: Ref) -> bool;
    static kCFBooleanTrue: Ref;
}

#[link(name = "Security", kind = "framework")]
extern "C" {
    fn SecItemCopyMatching(query: Ref, result: *mut Ref) -> i32;
    fn SecItemDelete(query: Ref) -> i32;
    static kSecClass: Ref;
    static kSecClassGenericPassword: Ref;
    static kSecAttrService: Ref;
    static kSecAttrAccount: Ref;
    static kSecAttrSynchronizable: Ref;
    static kSecAttrModificationDate: Ref;
    static kSecAttrCreationDate: Ref;
    static kSecReturnAttributes: Ref;
    static kSecReturnPersistentRef: Ref;
    static kSecValuePersistentRef: Ref;
    static kSecMatchLimit: Ref;
    static kSecMatchLimitAll: Ref;
    static kSecMatchItemList: Ref;
    static kSecUseAuthenticationUI: Ref;
    static kSecUseAuthenticationUIFail: Ref;
}

struct Owned(Ref);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CFRelease(self.0);
            }
        }
    }
}
impl Owned {
    fn string(value: &str) -> Result<Self> {
        let text = CString::new(value)?;
        let result =
            Self(unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), UTF8) });
        if result.0.is_null() {
            bail!("A Keychain query could not be constructed.");
        }
        Ok(result)
    }
    fn checked(value: Ref) -> Result<Self> {
        if value.is_null() {
            bail!("A Keychain query could not be constructed.");
        }
        Ok(Self(value))
    }
    fn data(value: &[u8]) -> Result<Self> {
        Self::checked(unsafe {
            CFDataCreate(std::ptr::null(), value.as_ptr(), value.len() as isize)
        })
    }
    fn array(values: &[Ref]) -> Result<Self> {
        Self::checked(unsafe {
            CFArrayCreate(
                std::ptr::null(),
                values.as_ptr(),
                values.len() as isize,
                std::ptr::null(),
            )
        })
    }
    fn dictionary(pairs: &[(Ref, Ref)]) -> Result<Self> {
        let keys: Vec<_> = pairs.iter().map(|p| p.0).collect();
        let values: Vec<_> = pairs.iter().map(|p| p.1).collect();
        Self::checked(unsafe {
            CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                pairs.len() as isize,
                std::ptr::null(),
                std::ptr::null(),
            )
        })
    }
}

pub(super) struct Item {
    reference: Vec<u8>,
    stamp: Vec<u8>,
    service: usize,
}
impl Item {
    pub fn fingerprint(&self) -> Vec<u8> {
        [
            vec![self.service as u8],
            (self.reference.len() as u64).to_le_bytes().to_vec(),
            self.reference.clone(),
            self.stamp.clone(),
        ]
        .concat()
    }
}

unsafe fn item(dict: Ref, service: usize) -> Result<Item> {
    if dict.is_null() || CFGetTypeID(dict) != CFDictionaryGetTypeID() {
        bail!("Keychain metadata is unavailable.");
    }
    let synchronized = CFDictionaryGetValue(dict, kSecAttrSynchronizable);
    if !synchronized.is_null()
        && (CFGetTypeID(synchronized) != CFBooleanGetTypeID() || CFBooleanGetValue(synchronized))
    {
        bail!("Synchronized or uncertain Keychain items are outside the cleanup scope.");
    }
    let reference = CFDictionaryGetValue(dict, kSecValuePersistentRef);
    if reference.is_null() || CFGetTypeID(reference) != CFDataGetTypeID() {
        bail!("Keychain item identity is unavailable.");
    }
    let length = CFDataGetLength(reference);
    if !(1..=8192).contains(&length) {
        bail!("Keychain item identity is unavailable.");
    }
    let mut stamp = Vec::new();
    for key in [kSecAttrCreationDate, kSecAttrModificationDate] {
        let date = CFDictionaryGetValue(dict, key);
        if date.is_null() || CFGetTypeID(date) != CFDateGetTypeID() {
            bail!("Keychain item freshness is unavailable.");
        }
        stamp.extend(CFDateGetAbsoluteTime(date).to_le_bytes());
    }
    Ok(Item {
        reference: std::slice::from_raw_parts(CFDataGetBytePtr(reference), length as usize)
            .to_vec(),
        stamp,
        service,
    })
}

fn query(pairs: &[(Ref, Ref)], service: usize) -> Result<Vec<Item>> {
    let dictionary = Owned::dictionary(pairs)?;
    let mut raw = std::ptr::null();
    let status = unsafe { SecItemCopyMatching(dictionary.0, &mut raw) };
    let result = Owned(raw);
    if status == -25300 {
        return Ok(vec![]);
    }
    if status != 0 || result.0.is_null() {
        bail!("Keychain inspection was unavailable or denied; no credential values were read.");
    }
    let mut items = Vec::new();
    unsafe {
        if CFGetTypeID(result.0) == CFArrayGetTypeID() {
            let length = CFArrayGetCount(result.0);
            if !(0..=100).contains(&length) {
                bail!("Too many matching Keychain items; cleanup is unavailable.");
            }
            for index in 0..length {
                items.push(item(CFArrayGetValueAtIndex(result.0, index), service)?);
            }
        } else {
            items.push(item(result.0, service)?);
        }
    }
    items.sort_by(|a, b| a.reference.cmp(&b.reference));
    Ok(items)
}

pub(super) fn inspect(account: &str) -> Result<Vec<Item>> {
    let account = Owned::string(account)?;
    let mut items = Vec::new();
    for (index, service) in SERVICES.iter().enumerate() {
        let service = Owned::string(service)?;
        let pairs = unsafe {
            [
                (kSecClass, kSecClassGenericPassword),
                (kSecAttrService, service.0),
                (kSecAttrAccount, account.0),
                (kSecMatchLimit, kSecMatchLimitAll),
                (kSecReturnAttributes, kCFBooleanTrue),
                (kSecReturnPersistentRef, kCFBooleanTrue),
                (kSecUseAuthenticationUI, kSecUseAuthenticationUIFail),
            ]
        };
        items.extend(query(&pairs, index)?);
    }
    let mut references = std::collections::BTreeSet::new();
    if items
        .iter()
        .any(|item| !references.insert(item.reference.clone()))
    {
        bail!("Keychain item identities are ambiguous; cleanup is unavailable.");
    }
    Ok(items)
}

fn inspect_reference(item: &Item) -> Result<Vec<Item>> {
    // The omitted sync attribute excludes synchronized items by default. Apple's
    // explicit sync-key queries cannot be combined with kSecMatchItemList.
    let reference = Owned::data(&item.reference)?;
    let list = Owned::array(&[reference.0])?;
    let pairs = unsafe {
        [
            (kSecClass, kSecClassGenericPassword),
            (kSecMatchItemList, list.0),
            (kSecReturnAttributes, kCFBooleanTrue),
            (kSecReturnPersistentRef, kCFBooleanTrue),
            (kSecUseAuthenticationUI, kSecUseAuthenticationUIFail),
        ]
    };
    query(&pairs, item.service)
}

pub(super) fn remove(item: &Item) -> Result<()> {
    let current = inspect_reference(item)?;
    if current.len() != 1 || current[0].fingerprint() != item.fingerprint() {
        bail!("A Keychain item changed; request a new preview.");
    }
    let reference = Owned::data(&item.reference)?;
    let list = Owned::array(&[reference.0])?;
    let dictionary = Owned::dictionary(&unsafe {
        [
            (kSecClass, kSecClassGenericPassword),
            (kSecMatchItemList, list.0),
            (kSecUseAuthenticationUI, kSecUseAuthenticationUIFail),
        ]
    })?;
    let status = unsafe { SecItemDelete(dictionary.0) };
    let remaining = inspect_reference(item)?;
    if status != 0 || !remaining.is_empty() {
        bail!("Keychain deletion or independent readback failed; request a new preview before retrying.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[link(name = "Security", kind = "framework")]
    extern "C" {
        fn SecKeychainCreate(
            path: *const c_char,
            length: u32,
            password: *const c_void,
            prompt: bool,
            access: Ref,
            keychain: *mut Ref,
        ) -> i32;
        fn SecKeychainDelete(keychain: Ref) -> i32;
        fn SecKeychainCopySearchList(list: *mut Ref) -> i32;
        fn SecItemAdd(attributes: Ref, result: *mut Ref) -> i32;
        static kSecUseKeychain: Ref;
        static kSecMatchSearchList: Ref;
        static kSecValueData: Ref;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFEqual(a: Ref, b: Ref) -> bool;
    }

    struct PrivateKeychain(Owned);
    impl Drop for PrivateKeychain {
        fn drop(&mut self) {
            assert_eq!(
                unsafe { SecKeychainDelete(self.0 .0) },
                0,
                "Private fixture Keychain deletion failed"
            );
        }
    }
    fn search_list() -> Owned {
        let mut list = std::ptr::null();
        assert_eq!(unsafe { SecKeychainCopySearchList(&mut list) }, 0);
        Owned::checked(list).unwrap()
    }
    fn add(keychain: &PrivateKeychain, service: &str, account: &str) {
        let service = Owned::string(service).unwrap();
        let account = Owned::string(account).unwrap();
        let bytes = Owned::data(b"SYNTHETIC_TEST_VALUE").unwrap();
        let attributes = Owned::dictionary(&unsafe {
            [
                (kSecClass, kSecClassGenericPassword),
                (kSecAttrService, service.0),
                (kSecAttrAccount, account.0),
                (kSecValueData, bytes.0),
                (kSecUseKeychain, keychain.0 .0),
            ]
        })
        .unwrap();
        assert_eq!(unsafe { SecItemAdd(attributes.0, std::ptr::null_mut()) }, 0);
    }
    fn inspect_private(keychain: &PrivateKeychain, service: &str, account: &str) -> Vec<Item> {
        let service = Owned::string(service).unwrap();
        let account = Owned::string(account).unwrap();
        let list = Owned::array(&[keychain.0 .0]).unwrap();
        query(
            &unsafe {
                [
                    (kSecClass, kSecClassGenericPassword),
                    (kSecAttrService, service.0),
                    (kSecAttrAccount, account.0),
                    (kSecMatchSearchList, list.0),
                    (kSecMatchLimit, kSecMatchLimitAll),
                    (kSecReturnAttributes, kCFBooleanTrue),
                    (kSecReturnPersistentRef, kCFBooleanTrue),
                    (kSecUseAuthenticationUI, kSecUseAuthenticationUIFail),
                ]
            },
            0,
        )
        .unwrap()
    }

    #[test]
    #[ignore = "Disposable macOS runner only; creates and removes a synthetic private Keychain"]
    fn private_keychain_persistent_reference_deletes_only_the_previewed_item() {
        assert_eq!(
            std::env::var("GITHUB_ACTIONS").as_deref(),
            Ok("true"),
            "Never run against a personal Keychain search list"
        );
        let before = search_list();
        let path = std::env::temp_dir().join(format!(
            "am-claude-keychain-{}.keychain-db",
            crate::store::uuid_v4()
        ));
        let name = CString::new(path.to_str().unwrap()).unwrap();
        let password = b"SYNTHETIC_FIXTURE_PASSWORD";
        let mut raw = std::ptr::null();
        assert_eq!(
            unsafe {
                SecKeychainCreate(
                    name.as_ptr(),
                    password.len() as u32,
                    password.as_ptr().cast(),
                    false,
                    std::ptr::null(),
                    &mut raw,
                )
            },
            0
        );
        let keychain = PrivateKeychain(Owned::checked(raw).unwrap());
        add(&keychain, SERVICES[1], "YOUR_TEST_ACCOUNT");
        add(&keychain, SERVICES[1], "YOUR_OTHER_ACCOUNT");
        add(&keychain, "YOUR_OTHER_SERVICE", "YOUR_TEST_ACCOUNT");
        let expected = inspect_private(&keychain, SERVICES[1], "YOUR_TEST_ACCOUNT");
        assert_eq!(expected.len(), 1);
        assert_eq!(
            inspect_reference(&expected[0]).unwrap()[0].fingerprint(),
            expected[0].fingerprint()
        );
        let other = inspect_private(&keychain, SERVICES[1], "YOUR_OTHER_ACCOUNT")[0].fingerprint();
        let unrelated =
            inspect_private(&keychain, "YOUR_OTHER_SERVICE", "YOUR_TEST_ACCOUNT")[0].fingerprint();
        remove(&expected[0]).unwrap();
        assert!(inspect_private(&keychain, SERVICES[1], "YOUR_TEST_ACCOUNT").is_empty());
        assert!(remove(&expected[0]).is_err());
        assert_eq!(
            inspect_private(&keychain, SERVICES[1], "YOUR_OTHER_ACCOUNT")[0].fingerprint(),
            other
        );
        assert_eq!(
            inspect_private(&keychain, "YOUR_OTHER_SERVICE", "YOUR_TEST_ACCOUNT")[0].fingerprint(),
            unrelated
        );
        drop(keychain);
        let after = search_list();
        assert!(
            unsafe { CFEqual(before.0, after.0) },
            "The Keychain search list changed"
        );
        assert!(!path.exists());
    }
}
