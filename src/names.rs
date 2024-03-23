use std::{collections::HashMap, fs, io, num::ParseIntError, path::Path, str::FromStr, sync::Arc};

#[derive(Debug)]
struct NamesInner {
    vendor: HashMap<VendorKey, Box<str>>,
    product: HashMap<ProductKey, Box<str>>,
    class: HashMap<ClassKey, Box<str>>,
    subclass: HashMap<SubclassKey, Box<str>>,
    protocol: HashMap<ProtocolKey, Box<str>>,
}

pub struct Names {
    inner: Arc<NamesInner>,
}

impl Names {
    pub fn vendor(&self, vendor: u16) -> Option<&str> {
        self.inner.vendor(vendor)
    }

    pub fn product(&self, vendor: u16, product: u16) -> Option<&str> {
        self.inner.product(vendor, product)
    }

    pub fn class(&self, class: u8) -> Option<&str> {
        self.inner.class(class)
    }

    pub fn subclass(&self, class: u8, subclass: u8) -> Option<&str> {
        self.inner.subclass(class, subclass)
    }

    pub fn protocol(&self, class: u8, subclass: u8, protocol: u8) -> Option<&str> {
        self.inner.protocol(class, subclass, protocol)
    }
}

impl NamesInner {
    pub fn new() -> Self {
        Self {
            vendor: HashMap::new(),
            product: HashMap::new(),
            class: HashMap::new(),
            subclass: HashMap::new(),
            protocol: HashMap::new(),
        }
    }

    pub fn vendor(&self, vendor: u16) -> Option<&str> {
        self.vendor.get(&VendorKey(vendor)).map(Box::as_ref)
    }

    pub fn product(&self, vendor: u16, product: u16) -> Option<&str> {
        self.product
            .get(&ProductKey { vendor, product })
            .map(Box::as_ref)
    }

    pub fn class(&self, class: u8) -> Option<&str> {
        self.class.get(&ClassKey(class)).map(Box::as_ref)
    }

    pub fn subclass(&self, class: u8, subclass: u8) -> Option<&str> {
        self.subclass
            .get(&SubclassKey { class, subclass })
            .map(Box::as_ref)
    }

    pub fn protocol(&self, class: u8, subclass: u8, protocol: u8) -> Option<&str> {
        self.protocol
            .get(&ProtocolKey {
                class,
                subclass,
                protocol,
            })
            .map(Box::as_ref)
    }
}

enum LastState {
    Start,
    Lang,
    Class(ClassKey),
    Subclass(SubclassKey),
    Vendor(VendorKey),
    Product(ProductKey),
    Hut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct VendorKey(u16);

impl FromStr for VendorKey {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(VendorKey(s.parse()?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProductKey {
    vendor: u16,
    product: u16,
}

impl ProductKey {
    fn from_str_and_vendor(s: &str, vendor: u16) -> Result<Self, ParseIntError> {
        Ok(ProductKey {
            vendor,
            product: s.parse()?,
        })
    }
}

impl std::hash::Hash for ProductKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let vendor = self.vendor as u32;
        let product = self.product as u32;
        let key: u32 = (vendor << 16) | product;
        key.hash(state)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ClassKey(u8);

impl FromStr for ClassKey {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(ClassKey(s.parse()?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SubclassKey {
    class: u8,
    subclass: u8,
}
impl SubclassKey {
    fn from_str_and_class(s: &str, class: u8) -> Result<SubclassKey, ParseIntError> {
        Ok(SubclassKey {
            class,
            subclass: s.parse()?,
        })
    }
}

impl std::hash::Hash for SubclassKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let class = self.class as u16;
        let subclass = self.class as u16;
        let key: u16 = (class << 8) | subclass;
        key.hash(state)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProtocolKey {
    class: u8,
    subclass: u8,
    protocol: u8,
}

impl ProtocolKey {
    fn from_str_class_and_subclass(
        s: &str,
        class: u8,
        subclass: u8,
    ) -> Result<ProtocolKey, ParseIntError> {
        Ok(ProtocolKey {
            class,
            subclass,
            protocol: s.parse()?,
        })
    }
}

impl std::hash::Hash for ProtocolKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let class = self.class as u32;
        let subclass = self.class as u32;
        let protocol = self.class as u32;
        let key: u32 = (class << 16) | (subclass << 8) | protocol;
        key.hash(state)
    }
}

fn can_skip(line: &str) -> bool {
    line.is_empty()
        || line.starts_with('#')
        || line.starts_with("PHYSDES ")
        || line.starts_with("PHY ")
        || line.starts_with("BIAS ")
        || line.starts_with("AT ")
        || line.starts_with("HCC ")
        || line.starts_with("HID ")
        || line.starts_with("R ")
        || line.starts_with("VT")
}

fn parse_value<F, T>(possible: &str, f: F) -> Option<(T, Box<str>)>
where
    F: Fn(&str) -> Result<T, ParseIntError>,
{
    possible
        .split_once(' ')
        .and_then(|(key_token, rest)| f(key_token).ok().map(|key| (key, Box::from(rest.trim()))))
}

fn parse_class(line: &str) -> Option<(ClassKey, Box<str>)> {
    parse_value(line.strip_prefix("C ")?, str::parse::<ClassKey>)
}

fn parse_product(line: &str, vendor: u16) -> Option<(ProductKey, Box<str>)> {
    parse_value(line.strip_prefix('\t')?, |token| {
        ProductKey::from_str_and_vendor(token, vendor)
    })
}

fn parse_subclass(line: &str, class: u8) -> Option<(SubclassKey, Box<str>)> {
    parse_value(line.strip_prefix('\t')?, |token| {
        SubclassKey::from_str_and_class(token, class)
    })
}

fn parse_protocol(line: &str, class: u8, subclass: u8) -> Option<(ProtocolKey, Box<str>)> {
    parse_value(line.strip_prefix("\t\t")?, |token| {
        ProtocolKey::from_str_class_and_subclass(token, class, subclass)
    })
}

fn parse_vendor(line: &str) -> Option<(VendorKey, Box<str>)> {
    parse_value(line, str::parse::<VendorKey>)
}

pub fn parse<P>(path: P) -> io::Result<Names>
where
    P: AsRef<Path>,
{
    let mut names = NamesInner::new();
    let mut last_state = LastState::Start;

    for (line, _num) in fs::read_to_string(path)?.lines().zip(1usize..) {
        if can_skip(line) {
            continue;
        }

        if line.contains("L ") {
            last_state = LastState::Lang;
            continue;
        }

        if let Some((key, text)) = parse_class(line) {
            if names.class.insert(key, text).is_some() {
                // Print message about duplicate vendor spec?
            }
            last_state = LastState::Class(key);
            continue;
        }

        if let Some((key, text)) = parse_vendor(line) {
            if names.vendor.insert(key, text).is_some() {
                // Etc...
            }
            last_state = LastState::Vendor(key);
            continue;
        }

        if line.contains("HUT ") {
            last_state = LastState::Hut;
            continue;
        }

        match last_state {
            LastState::Start | LastState::Lang | LastState::Hut => {}
            LastState::Class(ClassKey(class)) => {
                if let Some((key, text)) = parse_subclass(line, class) {
                    if names.subclass.insert(key, text).is_some() {
                        // Err...
                    }
                    last_state = LastState::Subclass(key);
                }
            }
            LastState::Subclass(SubclassKey { class, subclass }) => {
                if let Some((key, text)) = parse_subclass(line, class) {
                    if names.subclass.insert(key, text).is_some() {
                        // Err...
                    }
                    last_state = LastState::Subclass(key);
                } else if let Some((key, text)) = parse_protocol(line, class, subclass) {
                    if names.protocol.insert(key, text).is_some() {
                        // Err...
                    }
                }
            }
            LastState::Vendor(VendorKey(vendor))
            | LastState::Product(ProductKey { vendor, product: _ }) => {
                if let Some((key, text)) = parse_product(line, vendor) {
                    if names.product.insert(key, text).is_some() {
                        // Print message about duplicate vendor spec?
                    }
                    last_state = LastState::Product(key);
                }
            }
        }
    }

    Ok(Names {
        inner: Arc::from(names),
    })
}
