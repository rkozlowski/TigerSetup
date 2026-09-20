//! Windows Firewall rules through the `INetFwPolicy2` COM policy object
//! (`HNetCfg.FwPolicy2`), the API the Windows Defender Firewall console
//! itself uses. The `windows-sys` crate declares COM functions but no
//! interfaces, so the vtables this needs are declared here, in the shape
//! `netfw.h` gives them: every one is a dual interface, so `IDispatch`'s
//! four methods precede the interface's own.
//!
//! A rule is identified by its name, and Windows lets two rules share one:
//! the engine tells its own rule from a stranger's by the `Grouping` it
//! writes, and refuses to create beside, or remove past, a same-named rule
//! it does not own (`resource::firewall`). So [`Store::list`] returns every
//! rule with a name rather than the first.
//!
//! Adding and removing rules needs an administrator. An unelevated test
//! process cannot exercise the real policy object, so the store has a seam:
//! `TIGERSETUP_TEST_FIREWALL_STORE=<path>` names a JSON file that stands in
//! for the policy, with the same operations and the same semantics; the lab
//! proves the real one from an elevated machine-scope run.
//!
//! A rule the policy object accepted is not yet on the disk: the firewall
//! service keeps its rules in the `SYSTEM` hive, which Windows writes back
//! lazily like every hive, so a reset moments after the call loses the
//! rule while the journal already calls the operation applied. Every
//! mutation of the policy therefore ends with a flush of that hive
//! ([`flush_policy_store`]), the same answer the engine gives its own
//! registry values before the commit (`win::registry::flush`): the
//! mutation is durable before the record of it.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr;

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, SysAllocString, SysFreeString, SysStringLen};
use windows_sys::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, RegCloseKey, RegFlushKey, RegOpenKeyExW,
};
use windows_sys::core::{BSTR, GUID, HRESULT};

use crate::{Error, Result};

/// The environment variable that redirects the store to a JSON file.
pub const TEST_STORE_VARIABLE: &str = "TIGERSETUP_TEST_FIREWALL_STORE";

const CLSID_FW_POLICY2: GUID = GUID::from_u128(0xe2b3c97f_6ae1_41ac_817a_f6f92166d7dd);
const IID_INET_FW_POLICY2: GUID = GUID::from_u128(0x98325047_c671_4174_8d81_defcd3f03186);
const CLSID_FW_RULE: GUID = GUID::from_u128(0x2c5bc43e_3369_4c33_ab0c_be9469677af4);
const IID_INET_FW_RULE: GUID = GUID::from_u128(0xaf230d27_baba_4e42_aced_f524f22cfce2);
const IID_IENUM_VARIANT: GUID = GUID::from_u128(0x00020404_0000_0000_c000_000000000046);
/// `RPC_E_CHANGED_MODE`: COM was already initialised with another model;
/// the apartment is usable all the same.
const RPC_E_CHANGED_MODE: HRESULT = 0x8001_0106u32 as i32;
const S_FALSE: HRESULT = 1;
const VT_DISPATCH: u16 = 9;
const VARIANT_TRUE: i16 = -1;
const VARIANT_FALSE: i16 = 0;
const NET_FW_RULE_DIR_IN: i32 = 1;
const NET_FW_RULE_DIR_OUT: i32 = 2;
const NET_FW_ACTION_BLOCK: i32 = 0;
const NET_FW_ACTION_ALLOW: i32 = 1;
const NET_FW_IP_PROTOCOL_TCP: i32 = 6;
const NET_FW_IP_PROTOCOL_UDP: i32 = 17;
const NET_FW_IP_PROTOCOL_ANY: i32 = 256;
const NET_FW_PROFILE2_ALL: i32 = 0x7fff_ffff;

/// One firewall rule as TigerSetup reads and writes it: the attributes a
/// declared rule sets, spelled the way the manifest and the reports spell
/// them, so the same record serves the journal, the ownership table and a
/// comparison with what the machine holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Rule {
    pub name: String,
    pub description: String,
    /// The group TigerSetup files its rules under: the product's identity.
    /// This is how a rule of TigerSetup's is told from a stranger's with
    /// the same name.
    pub grouping: String,
    /// Absolute path of the program.
    pub program: String,
    /// `in` or `out`.
    pub direction: String,
    /// `allow` or `block`.
    pub action: String,
    /// `any`, `tcp` or `udp`.
    pub protocol: String,
    /// Local ports as written; empty means any.
    pub local_ports: String,
    pub enabled: bool,
}

impl Rule {
    /// The rule as the journal and the ownership table store it.
    pub fn serialize(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// A stored rule.
    pub fn deserialize(text: &str) -> Result<Rule> {
        serde_json::from_str(text).map_err(|err| {
            Error::new(
                "journal_inconsistent",
                format!("stored firewall rule is not readable: {err}"),
            )
        })
    }

    fn direction_code(&self) -> i32 {
        if self.direction == "out" {
            NET_FW_RULE_DIR_OUT
        } else {
            NET_FW_RULE_DIR_IN
        }
    }

    fn action_code(&self) -> i32 {
        if self.action == "block" {
            NET_FW_ACTION_BLOCK
        } else {
            NET_FW_ACTION_ALLOW
        }
    }

    fn protocol_code(&self) -> i32 {
        match self.protocol.as_str() {
            "tcp" => NET_FW_IP_PROTOCOL_TCP,
            "udp" => NET_FW_IP_PROTOCOL_UDP,
            _ => NET_FW_IP_PROTOCOL_ANY,
        }
    }
}

/// Where the rules live: the machine's firewall policy, or the test file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Store {
    Policy,
    File(PathBuf),
}

impl Store {
    /// The store this process uses: the file the seam names, else the
    /// policy object.
    pub fn from_env() -> Store {
        match std::env::var_os(TEST_STORE_VARIABLE) {
            Some(path) if !path.is_empty() => Store::File(PathBuf::from(path)),
            _ => Store::Policy,
        }
    }

    /// Whether this process may add and remove rules here: the policy
    /// object needs an administrator; the test file needs nothing.
    pub fn is_writable(&self) -> bool {
        match self {
            Store::Policy => crate::win::process::is_elevated(),
            Store::File(_) => true,
        }
    }

    /// Every rule carrying `name`, in the store's order.
    pub fn list(&self, name: &str) -> Result<Vec<Rule>> {
        match self {
            Store::Policy => Policy::open()?.list(name),
            Store::File(path) => Ok(read_file(path)?
                .into_iter()
                .filter(|r| r.name.eq_ignore_ascii_case(name))
                .collect()),
        }
    }

    /// Writes the rule: an existing rule of that name takes every
    /// attribute, otherwise the rule is added. The caller has already
    /// decided that a same-named rule, if any, is TigerSetup's own.
    pub fn put(&self, rule: &Rule) -> Result<()> {
        match self {
            Store::Policy => {
                Policy::open()?.put(rule)?;
                flush_policy_store()
            }
            Store::File(path) => {
                let mut rules = read_file(path)?;
                match rules
                    .iter_mut()
                    .find(|r| r.name.eq_ignore_ascii_case(&rule.name))
                {
                    Some(existing) => *existing = rule.clone(),
                    None => rules.push(rule.clone()),
                }
                write_file(path, &rules)
            }
        }
    }

    /// Removes the rule of that name; `false` when there was none.
    pub fn remove(&self, name: &str) -> Result<bool> {
        match self {
            Store::Policy => {
                let removed = Policy::open()?.remove(name)?;
                if removed {
                    flush_policy_store()?;
                }
                Ok(removed)
            }
            Store::File(path) => {
                let mut rules = read_file(path)?;
                let before = rules.len();
                rules.retain(|r| !r.name.eq_ignore_ascii_case(name));
                if rules.len() == before {
                    return Ok(false);
                }
                write_file(path, &rules)?;
                Ok(true)
            }
        }
    }
}

/// The key under which the firewall service keeps its rules: a key of the
/// `SYSTEM` hive, which is what flushing it writes to the disk.
const POLICY_STORE_KEY: &str =
    r"SYSTEM\CurrentControlSet\Services\SharedAccess\Parameters\FirewallPolicy";

/// Writes the firewall service's hive to the disk, so that a rule the
/// policy object just accepted survives a reset before the journal records
/// it as applied. `RegFlushKey` flushes the hive a key belongs to; the key
/// only has to be open for query, which an administrator — the only caller
/// that gets this far — has.
fn flush_policy_store() -> Result<()> {
    let subkey: Vec<u16> = POLICY_STORE_KEY
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut key: HKEY = ptr::null_mut();
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if opened != ERROR_SUCCESS {
        return Err(Error::new(
            "firewall_error",
            format!(
                "cannot open the firewall policy store to flush it: {}",
                std::io::Error::from_raw_os_error(opened as i32)
            ),
        ));
    }
    let flushed = unsafe { RegFlushKey(key) };
    unsafe { RegCloseKey(key) };
    if flushed != ERROR_SUCCESS {
        return Err(Error::new(
            "firewall_error",
            format!(
                "cannot flush the firewall policy store: {}",
                std::io::Error::from_raw_os_error(flushed as i32)
            ),
        ));
    }
    Ok(())
}

fn read_file(path: &Path) -> Result<Vec<Rule>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(path)?;
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&text).map_err(|err| {
        Error::new(
            "firewall_error",
            format!(
                "the firewall test store {} is not readable: {err}",
                path.display()
            ),
        )
    })
}

fn write_file(path: &Path, rules: &[Rule]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(rules).unwrap_or_default(),
    )?;
    Ok(())
}

fn com_error(what: &str, hr: HRESULT) -> Error {
    Error::new(
        "firewall_error",
        format!("{what}: HRESULT 0x{:08x}", hr as u32),
    )
}

// --- COM plumbing ---------------------------------------------------------

#[repr(C)]
struct IUnknownVtbl {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

/// `IDispatch`'s four methods, which every firewall interface carries
/// before its own and this module never calls.
#[repr(C)]
struct IDispatchVtbl {
    base: IUnknownVtbl,
    get_type_info_count: *const c_void,
    get_type_info: *const c_void,
    get_ids_of_names: *const c_void,
    invoke: *const c_void,
}

type BstrGetter = unsafe extern "system" fn(*mut c_void, *mut BSTR) -> HRESULT;
type BstrSetter = unsafe extern "system" fn(*mut c_void, BSTR) -> HRESULT;
type LongGetter = unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT;
type LongSetter = unsafe extern "system" fn(*mut c_void, i32) -> HRESULT;
type BoolGetter = unsafe extern "system" fn(*mut c_void, *mut i16) -> HRESULT;
type BoolSetter = unsafe extern "system" fn(*mut c_void, i16) -> HRESULT;

#[repr(C)]
struct INetFwPolicy2Vtbl {
    base: IDispatchVtbl,
    get_current_profile_types: *const c_void,
    get_firewall_enabled: *const c_void,
    put_firewall_enabled: *const c_void,
    get_excluded_interfaces: *const c_void,
    put_excluded_interfaces: *const c_void,
    get_block_all_inbound_traffic: *const c_void,
    put_block_all_inbound_traffic: *const c_void,
    get_notifications_disabled: *const c_void,
    put_notifications_disabled: *const c_void,
    get_unicast_responses_disabled: *const c_void,
    put_unicast_responses_disabled: *const c_void,
    get_rules: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    // The remaining methods are not used.
}

#[repr(C)]
struct INetFwRulesVtbl {
    base: IDispatchVtbl,
    get_count: LongGetter,
    add: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    remove: BstrSetter,
    item: unsafe extern "system" fn(*mut c_void, BSTR, *mut *mut c_void) -> HRESULT,
    get_new_enum: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct INetFwRuleVtbl {
    base: IDispatchVtbl,
    get_name: BstrGetter,
    put_name: BstrSetter,
    get_description: BstrGetter,
    put_description: BstrSetter,
    get_application_name: BstrGetter,
    put_application_name: BstrSetter,
    get_service_name: BstrGetter,
    put_service_name: BstrSetter,
    get_protocol: LongGetter,
    put_protocol: LongSetter,
    get_local_ports: BstrGetter,
    put_local_ports: BstrSetter,
    get_remote_ports: BstrGetter,
    put_remote_ports: BstrSetter,
    get_local_addresses: BstrGetter,
    put_local_addresses: BstrSetter,
    get_remote_addresses: BstrGetter,
    put_remote_addresses: BstrSetter,
    get_icmp_types_and_codes: BstrGetter,
    put_icmp_types_and_codes: BstrSetter,
    get_direction: LongGetter,
    put_direction: LongSetter,
    get_interfaces: *const c_void,
    put_interfaces: *const c_void,
    get_interface_types: BstrGetter,
    put_interface_types: BstrSetter,
    get_enabled: BoolGetter,
    put_enabled: BoolSetter,
    get_grouping: BstrGetter,
    put_grouping: BstrSetter,
    get_profiles: LongGetter,
    put_profiles: LongSetter,
    get_edge_traversal: BoolGetter,
    put_edge_traversal: BoolSetter,
    get_action: LongGetter,
    put_action: LongSetter,
}

/// `VARIANT` as far as an `IDispatch` element needs it.
#[repr(C)]
struct Variant {
    vt: u16,
    reserved: [u16; 3],
    pointer: *mut c_void,
    padding: usize,
}

#[repr(C)]
struct IEnumVariantVtbl {
    base: IUnknownVtbl,
    next: unsafe extern "system" fn(*mut c_void, u32, *mut Variant, *mut u32) -> HRESULT,
    skip: *const c_void,
    reset: *const c_void,
    clone: *const c_void,
}

/// A COM apartment for the current thread, released on drop.
struct Apartment {
    uninitialise: bool,
}

impl Apartment {
    fn enter() -> Result<Apartment> {
        let hr = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        if hr == RPC_E_CHANGED_MODE {
            return Ok(Apartment {
                uninitialise: false,
            });
        }
        if hr < 0 {
            return Err(com_error("cannot initialise COM for the firewall", hr));
        }
        Ok(Apartment { uninitialise: true })
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.uninitialise {
            unsafe { CoUninitialize() };
        }
    }
}

/// A `BSTR` allocated by this module, freed on drop.
struct Bstr(BSTR);

impl Bstr {
    fn new(text: &str) -> Bstr {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        Bstr(unsafe { SysAllocString(wide.as_ptr()) })
    }
}

impl Drop for Bstr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { SysFreeString(self.0) };
        }
    }
}

/// The text of a `BSTR` a getter returned, which is then freed.
fn take_bstr(bstr: BSTR) -> String {
    if bstr.is_null() {
        return String::new();
    }
    let length = unsafe { SysStringLen(bstr) } as usize;
    let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(bstr, length) });
    unsafe { SysFreeString(bstr) };
    text
}

/// A COM interface pointer, released on drop.
struct Com(*mut c_void);

impl Com {
    fn unknown(&self) -> &IUnknownVtbl {
        unsafe { &*(self.0 as *mut *mut IUnknownVtbl).read() }
    }

    fn query(&self, iid: &GUID, what: &str) -> Result<Com> {
        let mut out: *mut c_void = ptr::null_mut();
        let hr = unsafe { (self.unknown().query_interface)(self.0, iid, &mut out) };
        if hr < 0 || out.is_null() {
            return Err(com_error(what, hr));
        }
        Ok(Com(out))
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { (self.unknown().release)(self.0) };
        }
    }
}

/// The firewall policy and its rule collection. Fields drop in declaration
/// order, so the interfaces are released before the apartment that owns
/// them is uninitialised — a release after `CoUninitialize` is an access
/// violation the optimised build does not forgive.
struct Policy {
    rules: Com,
    _policy: Com,
    _apartment: Apartment,
}

impl Policy {
    fn open() -> Result<Policy> {
        let apartment = Apartment::enter()?;
        let mut policy: *mut c_void = ptr::null_mut();
        let hr = unsafe {
            CoCreateInstance(
                &CLSID_FW_POLICY2,
                ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_INET_FW_POLICY2,
                &mut policy,
            )
        };
        if hr < 0 || policy.is_null() {
            return Err(com_error("cannot open the firewall policy", hr));
        }
        let policy = Com(policy);
        let vtbl = unsafe { &*(policy.0 as *mut *mut INetFwPolicy2Vtbl).read() };
        let mut rules: *mut c_void = ptr::null_mut();
        let hr = unsafe { (vtbl.get_rules)(policy.0, &mut rules) };
        if hr < 0 || rules.is_null() {
            return Err(com_error("cannot read the firewall rules", hr));
        }
        Ok(Policy {
            rules: Com(rules),
            _policy: policy,
            _apartment: apartment,
        })
    }

    fn rules_vtbl(&self) -> &INetFwRulesVtbl {
        unsafe { &*(self.rules.0 as *mut *mut INetFwRulesVtbl).read() }
    }

    /// Every rule with the name, walked through the collection's enumerator
    /// because `Item` answers with the first only.
    fn list(&self, name: &str) -> Result<Vec<Rule>> {
        let mut enumerator: *mut c_void = ptr::null_mut();
        let hr = unsafe { (self.rules_vtbl().get_new_enum)(self.rules.0, &mut enumerator) };
        if hr < 0 || enumerator.is_null() {
            return Err(com_error("cannot enumerate the firewall rules", hr));
        }
        let enumerator =
            Com(enumerator).query(&IID_IENUM_VARIANT, "cannot enumerate the firewall rules")?;
        let vtbl = unsafe { &*(enumerator.0 as *mut *mut IEnumVariantVtbl).read() };
        let mut out = Vec::new();
        loop {
            let mut item = Variant {
                vt: 0,
                reserved: [0; 3],
                pointer: ptr::null_mut(),
                padding: 0,
            };
            let mut fetched = 0u32;
            let hr = unsafe { (vtbl.next)(enumerator.0, 1, &mut item, &mut fetched) };
            if hr < 0 {
                return Err(com_error("cannot enumerate the firewall rules", hr));
            }
            if hr == S_FALSE || fetched == 0 || item.pointer.is_null() {
                break;
            }
            if item.vt != VT_DISPATCH {
                continue;
            }
            let dispatch = Com(item.pointer);
            let rule = dispatch.query(&IID_INET_FW_RULE, "cannot read a firewall rule")?;
            let read = FwRule(rule);
            if read.text(read.vtbl().get_name).eq_ignore_ascii_case(name) {
                out.push(read.to_rule());
            }
        }
        Ok(out)
    }

    fn put(&self, rule: &Rule) -> Result<()> {
        let name = Bstr::new(&rule.name);
        let mut existing: *mut c_void = ptr::null_mut();
        let hr = unsafe { (self.rules_vtbl().item)(self.rules.0, name.0, &mut existing) };
        if hr >= 0 && !existing.is_null() {
            let target = FwRule(Com(existing));
            return target.write(rule);
        }
        let mut created: *mut c_void = ptr::null_mut();
        let hr = unsafe {
            CoCreateInstance(
                &CLSID_FW_RULE,
                ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_INET_FW_RULE,
                &mut created,
            )
        };
        if hr < 0 || created.is_null() {
            return Err(com_error("cannot create a firewall rule", hr));
        }
        let target = FwRule(Com(created));
        target.write(rule)?;
        let hr = unsafe { (self.rules_vtbl().add)(self.rules.0, target.0.0) };
        if hr < 0 {
            return Err(com_error(
                &format!("cannot add the firewall rule {:?}", rule.name),
                hr,
            ));
        }
        Ok(())
    }

    fn remove(&self, name: &str) -> Result<bool> {
        if self.list(name)?.is_empty() {
            return Ok(false);
        }
        let bstr = Bstr::new(name);
        let hr = unsafe { (self.rules_vtbl().remove)(self.rules.0, bstr.0) };
        if hr < 0 {
            return Err(com_error(
                &format!("cannot remove the firewall rule {name:?}"),
                hr,
            ));
        }
        Ok(true)
    }
}

/// One `INetFwRule`.
struct FwRule(Com);

impl FwRule {
    fn vtbl(&self) -> &INetFwRuleVtbl {
        unsafe { &*(self.0.0 as *mut *mut INetFwRuleVtbl).read() }
    }

    fn text(&self, getter: BstrGetter) -> String {
        let mut bstr: BSTR = ptr::null_mut();
        let hr = unsafe { getter(self.0.0, &mut bstr) };
        if hr < 0 {
            return String::new();
        }
        take_bstr(bstr)
    }

    fn long(&self, getter: LongGetter) -> i32 {
        let mut value = 0i32;
        let hr = unsafe { getter(self.0.0, &mut value) };
        if hr < 0 { 0 } else { value }
    }

    fn boolean(&self, getter: BoolGetter) -> bool {
        let mut value = VARIANT_FALSE;
        let hr = unsafe { getter(self.0.0, &mut value) };
        hr >= 0 && value != VARIANT_FALSE
    }

    fn set_text(&self, what: &str, setter: BstrSetter, value: &str) -> Result<()> {
        let bstr = Bstr::new(value);
        let hr = unsafe { setter(self.0.0, bstr.0) };
        if hr < 0 {
            return Err(com_error(
                &format!("cannot set the {what} of a firewall rule"),
                hr,
            ));
        }
        Ok(())
    }

    fn set_long(&self, what: &str, setter: LongSetter, value: i32) -> Result<()> {
        let hr = unsafe { setter(self.0.0, value) };
        if hr < 0 {
            return Err(com_error(
                &format!("cannot set the {what} of a firewall rule"),
                hr,
            ));
        }
        Ok(())
    }

    fn to_rule(&self) -> Rule {
        let vtbl = self.vtbl();
        Rule {
            name: self.text(vtbl.get_name),
            description: self.text(vtbl.get_description),
            grouping: self.text(vtbl.get_grouping),
            program: self.text(vtbl.get_application_name),
            direction: match self.long(vtbl.get_direction) {
                NET_FW_RULE_DIR_OUT => "out".into(),
                _ => "in".into(),
            },
            action: match self.long(vtbl.get_action) {
                NET_FW_ACTION_BLOCK => "block".into(),
                _ => "allow".into(),
            },
            protocol: match self.long(vtbl.get_protocol) {
                NET_FW_IP_PROTOCOL_TCP => "tcp".into(),
                NET_FW_IP_PROTOCOL_UDP => "udp".into(),
                _ => "any".into(),
            },
            local_ports: {
                let ports = self.text(vtbl.get_local_ports);
                if ports == "*" { String::new() } else { ports }
            },
            enabled: self.boolean(vtbl.get_enabled),
        }
    }

    /// Writes every attribute of `rule`, in the order the policy object
    /// accepts them: the protocol before the ports, because ports are
    /// refused for a rule whose protocol does not have them.
    fn write(&self, rule: &Rule) -> Result<()> {
        let vtbl = self.vtbl();
        self.set_text("name", vtbl.put_name, &rule.name)?;
        self.set_text("description", vtbl.put_description, &rule.description)?;
        self.set_text("grouping", vtbl.put_grouping, &rule.grouping)?;
        self.set_text("program", vtbl.put_application_name, &rule.program)?;
        self.set_long("direction", vtbl.put_direction, rule.direction_code())?;
        self.set_long("protocol", vtbl.put_protocol, rule.protocol_code())?;
        if rule.protocol_code() != NET_FW_IP_PROTOCOL_ANY {
            let ports = if rule.local_ports.is_empty() {
                "*"
            } else {
                rule.local_ports.as_str()
            };
            self.set_text("local ports", vtbl.put_local_ports, ports)?;
        }
        self.set_long("profiles", vtbl.put_profiles, NET_FW_PROFILE2_ALL)?;
        self.set_long("action", vtbl.put_action, rule.action_code())?;
        let hr = unsafe {
            (vtbl.put_enabled)(
                self.0.0,
                if rule.enabled {
                    VARIANT_TRUE
                } else {
                    VARIANT_FALSE
                },
            )
        };
        if hr < 0 {
            return Err(com_error("cannot enable a firewall rule", hr));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> Rule {
        Rule {
            name: name.into(),
            description: "test".into(),
            grouping: "TigerSetup: T".into(),
            program: "C:\\P\\app.exe".into(),
            direction: "in".into(),
            action: "allow".into(),
            protocol: "tcp".into(),
            local_ports: "8000-8010".into(),
            enabled: true,
        }
    }

    #[test]
    fn rules_serialize_round_trip() {
        let rule = sample("Test rule");
        assert_eq!(Rule::deserialize(&rule.serialize()).unwrap(), rule);
        assert_eq!(rule.direction_code(), NET_FW_RULE_DIR_IN);
        assert_eq!(rule.protocol_code(), NET_FW_IP_PROTOCOL_TCP);
        assert!(Rule::deserialize("nonsense").is_err());
    }

    #[test]
    fn the_file_store_lists_puts_and_removes_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::File(dir.path().join("fw").join("rules.json"));
        assert!(store.is_writable());
        assert!(store.list("Test rule").unwrap().is_empty());
        store.put(&sample("Test rule")).unwrap();
        store.put(&sample("Other")).unwrap();
        assert_eq!(store.list("test RULE").unwrap(), vec![sample("Test rule")]);
        let mut changed = sample("Test rule");
        changed.enabled = false;
        store.put(&changed).unwrap();
        assert_eq!(store.list("Test rule").unwrap(), vec![changed]);
        assert!(store.remove("Test rule").unwrap());
        assert!(!store.remove("Test rule").unwrap());
        assert_eq!(store.list("Other").unwrap().len(), 1);
    }

    /// Reading the real policy needs no administrator; this proves the
    /// vtables line up with the policy object, which the elevated lab run
    /// then exercises for writing.
    #[test]
    fn the_policy_object_answers_a_read() {
        let policy = Policy::open().expect("the firewall policy opens");
        let rules = policy
            .list("TigerSetup-test-rule-that-does-not-exist")
            .expect("the rules enumerate");
        assert!(rules.is_empty());
    }
}
