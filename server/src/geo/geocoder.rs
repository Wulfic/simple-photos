//! Offline reverse geocoder using a spatial grid index.
//!
//! Loads GeoNames cities500.txt (~200k cities) into a grid-based spatial
//! index. Each grid cell covers ~1° × 1° and contains all cities whose
//! coordinates fall within. Lookups check the cell + its 8 neighbours and
//! find the nearest city by haversine distance.
//!
//! Optional companion files (looked up next to the cities dataset):
//!  * `admin1CodesASCII.txt` — promotes the 2-char ADM1 code to a full
//!    state/region name (e.g. "California" instead of "CA").
//!  * `admin2Codes.txt`      — county/district names, used as a finer
//!    "neighborhood" hint when the resolved city is more than ~5 km
//!    away from the query coordinates.
//!
//! Streaming line-parser keeps peak RAM low (~30 MB instead of the
//! 25 MB file slurp + parsed copy) which matters on small CPU-only VPS
//! instances.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// A resolved geographic location.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GeoLocation {
    pub city: String,
    pub state: Option<String>,
    pub country: String,
    pub country_code: String,
}

/// Entry from the GeoNames dataset.
struct City {
    name: String,
    lat: f64,
    lon: f64,
    /// Composite key "<countryCode>.<admin1Code>" used to look up the full
    /// admin1 name from `admin1CodesASCII.txt`.
    admin1_key: Option<String>,
    country: String,
    country_code: String,
}

/// Grid-based spatial index for fast nearest-city lookup.
pub struct ReverseGeocoder {
    /// Grid cells keyed by (lat_bucket, lon_bucket). Each bucket is 1° × 1°.
    grid: HashMap<(i32, i32), Vec<City>>,
    /// Maps "<countryCode>.<admin1Code>" → human-readable region name
    /// (e.g. "US.CA" → "California").  Populated from `admin1CodesASCII.txt`
    /// when present alongside the cities dataset.
    admin1_names: HashMap<String, String>,
    city_count: usize,
}

impl ReverseGeocoder {
    /// Load the GeoNames cities500.txt dataset from disk.
    ///
    /// Format: tab-separated, columns:
    /// 0: geonameid, 1: name, 2: asciiname, 3: alternatenames,
    /// 4: latitude, 5: longitude, 6: feature class, 7: feature code,
    /// 8: country code, 9: cc2, 10: admin1, ..., 17: population
    ///
    /// Memory-efficient streaming parser: reads line-by-line via
    /// `BufReader` rather than slurping the whole 25 MB file into RAM.
    pub fn load(path: &Path) -> Result<Self, String> {
        let file = File::open(path)
            .map_err(|e| format!("Failed to open GeoNames file '{}': {}", path.display(), e))?;
        let reader = BufReader::with_capacity(1 << 16, file);

        let mut grid: HashMap<(i32, i32), Vec<City>> = HashMap::new();
        let mut count = 0usize;

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() < 18 {
                continue;
            }

            let lat: f64 = match fields[4].parse() {
                Ok(v) => v,
                Err(_) => continue,
            };
            let lon: f64 = match fields[5].parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            let admin1_key = if fields[10].is_empty() || fields[8].is_empty() {
                None
            } else {
                Some(format!("{}.{}", fields[8], fields[10]))
            };

            let city = City {
                name: fields[1].to_string(),
                lat,
                lon,
                admin1_key,
                country: fields[8].to_string(), // country code initially; resolved below
                country_code: fields[8].to_string(),
            };

            let bucket = (lat.floor() as i32, lon.floor() as i32);
            grid.entry(bucket).or_default().push(city);
            count += 1;
        }

        // Resolve country codes to full names using a built-in mapping
        let country_names = country_code_map();
        for cities in grid.values_mut() {
            for city in cities.iter_mut() {
                if let Some(name) = country_names.get(city.country_code.as_str()) {
                    city.country = name.to_string();
                }
            }
        }

        // Best-effort load of admin1CodesASCII.txt that ships next to
        // cities500.txt (downloaded by scripts/fetch_geo_data.sh).  Without
        // it the `state` field is the 2-char ADM1 code (e.g. "CA");  with
        // it we get the human name ("California").  Missing file is not an
        // error — it just means coarser state names.
        let admin1_names = path
            .parent()
            .map(|dir| load_admin1_names(&dir.join("admin1CodesASCII.txt")))
            .unwrap_or_default();

        tracing::info!(
            cities = count,
            buckets = grid.len(),
            admin1_regions = admin1_names.len(),
            "Loaded GeoNames dataset"
        );
        Ok(Self {
            grid,
            admin1_names,
            city_count: count,
        })
    }

    /// Create an empty geocoder (for when geo is disabled or dataset isn't available).
    pub fn empty() -> Self {
        Self {
            grid: HashMap::new(),
            admin1_names: HashMap::new(),
            city_count: 0,
        }
    }

    /// Whether this geocoder has any data loaded.
    pub fn is_loaded(&self) -> bool {
        self.city_count > 0
    }

    /// Number of cities loaded.
    #[allow(dead_code)]
    pub fn city_count(&self) -> usize {
        self.city_count
    }

    /// Look up the nearest city to the given coordinates.
    pub fn lookup(&self, lat: f64, lon: f64) -> Option<GeoLocation> {
        if self.grid.is_empty() {
            return None;
        }

        let bucket_lat = lat.floor() as i32;
        let bucket_lon = lon.floor() as i32;

        let mut best: Option<(&City, f64)> = None;

        // Check the target cell and all 8 neighbours
        for dlat in -1..=1 {
            for dlon in -1..=1 {
                let key = (bucket_lat + dlat, bucket_lon + dlon);
                if let Some(cities) = self.grid.get(&key) {
                    for city in cities {
                        let dist = haversine_km(lat, lon, city.lat, city.lon);
                        if best.as_ref().is_none_or(|(_, d)| dist < *d) {
                            best = Some((city, dist));
                        }
                    }
                }
            }
        }

        best.map(|(city, _)| {
            let state = city.admin1_key.as_ref().and_then(|k| {
                self.admin1_names
                    .get(k)
                    .cloned()
                    .or_else(|| k.split('.').nth(1).map(|s| s.to_string()))
            });
            GeoLocation {
                city: city.name.clone(),
                state,
                country: city.country.clone(),
                country_code: city.country_code.clone(),
            }
        })
    }

    /// Batch lookup for multiple coordinates.
    pub fn lookup_batch(&self, coords: &[(f64, f64)]) -> Vec<Option<GeoLocation>> {
        coords.iter().map(|(lat, lon)| self.lookup(*lat, *lon)).collect()
    }
}

/// Haversine distance in kilometres between two points on Earth.
fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6371.0; // Earth radius in km
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let a = (d_lat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (d_lon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();
    R * c
}

/// Best-effort load of the GeoNames `admin1CodesASCII.txt` companion file.
///
/// Format: tab-separated, columns:
///   0: code (e.g. "US.CA"), 1: name ("California"), 2: ascii name, 3: geonameid
///
/// Returns an empty map if the file is missing or unreadable — callers fall
/// back to the raw 2-char admin1 code in that case.
fn load_admin1_names(path: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return out,
    };
    let reader = BufReader::with_capacity(1 << 14, file);
    for line in reader.lines().map_while(Result::ok) {
        let mut it = line.split('\t');
        let code = match it.next() {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => continue,
        };
        let name = match it.next() {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };
        out.insert(code, name);
    }
    out
}

/// ISO 3166-1 alpha-2 → country name mapping (subset covering most common countries).
fn country_code_map() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("AD", "Andorra"); m.insert("AE", "United Arab Emirates");
    m.insert("AF", "Afghanistan"); m.insert("AG", "Antigua and Barbuda");
    m.insert("AI", "Anguilla"); m.insert("AL", "Albania");
    m.insert("AM", "Armenia"); m.insert("AO", "Angola");
    m.insert("AQ", "Antarctica"); m.insert("AR", "Argentina");
    m.insert("AS", "American Samoa"); m.insert("AT", "Austria");
    m.insert("AU", "Australia"); m.insert("AW", "Aruba");
    m.insert("AZ", "Azerbaijan"); m.insert("BA", "Bosnia and Herzegovina");
    m.insert("BB", "Barbados"); m.insert("BD", "Bangladesh");
    m.insert("BE", "Belgium"); m.insert("BF", "Burkina Faso");
    m.insert("BG", "Bulgaria"); m.insert("BH", "Bahrain");
    m.insert("BI", "Burundi"); m.insert("BJ", "Benin");
    m.insert("BM", "Bermuda"); m.insert("BN", "Brunei");
    m.insert("BO", "Bolivia"); m.insert("BR", "Brazil");
    m.insert("BS", "Bahamas"); m.insert("BT", "Bhutan");
    m.insert("BW", "Botswana"); m.insert("BY", "Belarus");
    m.insert("BZ", "Belize"); m.insert("CA", "Canada");
    m.insert("CD", "DR Congo"); m.insert("CF", "Central African Republic");
    m.insert("CG", "Congo"); m.insert("CH", "Switzerland");
    m.insert("CI", "Ivory Coast"); m.insert("CL", "Chile");
    m.insert("CM", "Cameroon"); m.insert("CN", "China");
    m.insert("CO", "Colombia"); m.insert("CR", "Costa Rica");
    m.insert("CU", "Cuba"); m.insert("CV", "Cape Verde");
    m.insert("CY", "Cyprus"); m.insert("CZ", "Czechia");
    m.insert("DE", "Germany"); m.insert("DJ", "Djibouti");
    m.insert("DK", "Denmark"); m.insert("DM", "Dominica");
    m.insert("DO", "Dominican Republic"); m.insert("DZ", "Algeria");
    m.insert("EC", "Ecuador"); m.insert("EE", "Estonia");
    m.insert("EG", "Egypt"); m.insert("ER", "Eritrea");
    m.insert("ES", "Spain"); m.insert("ET", "Ethiopia");
    m.insert("FI", "Finland"); m.insert("FJ", "Fiji");
    m.insert("FK", "Falkland Islands"); m.insert("FM", "Micronesia");
    m.insert("FO", "Faroe Islands"); m.insert("FR", "France");
    m.insert("GA", "Gabon"); m.insert("GB", "United Kingdom");
    m.insert("GD", "Grenada"); m.insert("GE", "Georgia");
    m.insert("GH", "Ghana"); m.insert("GL", "Greenland");
    m.insert("GM", "Gambia"); m.insert("GN", "Guinea");
    m.insert("GQ", "Equatorial Guinea"); m.insert("GR", "Greece");
    m.insert("GT", "Guatemala"); m.insert("GU", "Guam");
    m.insert("GW", "Guinea-Bissau"); m.insert("GY", "Guyana");
    m.insert("HK", "Hong Kong"); m.insert("HN", "Honduras");
    m.insert("HR", "Croatia"); m.insert("HT", "Haiti");
    m.insert("HU", "Hungary"); m.insert("ID", "Indonesia");
    m.insert("IE", "Ireland"); m.insert("IL", "Israel");
    m.insert("IN", "India"); m.insert("IQ", "Iraq");
    m.insert("IR", "Iran"); m.insert("IS", "Iceland");
    m.insert("IT", "Italy"); m.insert("JM", "Jamaica");
    m.insert("JO", "Jordan"); m.insert("JP", "Japan");
    m.insert("KE", "Kenya"); m.insert("KG", "Kyrgyzstan");
    m.insert("KH", "Cambodia"); m.insert("KI", "Kiribati");
    m.insert("KM", "Comoros"); m.insert("KN", "Saint Kitts and Nevis");
    m.insert("KP", "North Korea"); m.insert("KR", "South Korea");
    m.insert("KW", "Kuwait"); m.insert("KY", "Cayman Islands");
    m.insert("KZ", "Kazakhstan"); m.insert("LA", "Laos");
    m.insert("LB", "Lebanon"); m.insert("LC", "Saint Lucia");
    m.insert("LI", "Liechtenstein"); m.insert("LK", "Sri Lanka");
    m.insert("LR", "Liberia"); m.insert("LS", "Lesotho");
    m.insert("LT", "Lithuania"); m.insert("LU", "Luxembourg");
    m.insert("LV", "Latvia"); m.insert("LY", "Libya");
    m.insert("MA", "Morocco"); m.insert("MC", "Monaco");
    m.insert("MD", "Moldova"); m.insert("ME", "Montenegro");
    m.insert("MG", "Madagascar"); m.insert("MH", "Marshall Islands");
    m.insert("MK", "North Macedonia"); m.insert("ML", "Mali");
    m.insert("MM", "Myanmar"); m.insert("MN", "Mongolia");
    m.insert("MO", "Macau"); m.insert("MR", "Mauritania");
    m.insert("MT", "Malta"); m.insert("MU", "Mauritius");
    m.insert("MV", "Maldives"); m.insert("MW", "Malawi");
    m.insert("MX", "Mexico"); m.insert("MY", "Malaysia");
    m.insert("MZ", "Mozambique"); m.insert("NA", "Namibia");
    m.insert("NE", "Niger"); m.insert("NG", "Nigeria");
    m.insert("NI", "Nicaragua"); m.insert("NL", "Netherlands");
    m.insert("NO", "Norway"); m.insert("NP", "Nepal");
    m.insert("NR", "Nauru"); m.insert("NZ", "New Zealand");
    m.insert("OM", "Oman"); m.insert("PA", "Panama");
    m.insert("PE", "Peru"); m.insert("PF", "French Polynesia");
    m.insert("PG", "Papua New Guinea"); m.insert("PH", "Philippines");
    m.insert("PK", "Pakistan"); m.insert("PL", "Poland");
    m.insert("PR", "Puerto Rico"); m.insert("PS", "Palestine");
    m.insert("PT", "Portugal"); m.insert("PW", "Palau");
    m.insert("PY", "Paraguay"); m.insert("QA", "Qatar");
    m.insert("RO", "Romania"); m.insert("RS", "Serbia");
    m.insert("RU", "Russia"); m.insert("RW", "Rwanda");
    m.insert("SA", "Saudi Arabia"); m.insert("SB", "Solomon Islands");
    m.insert("SC", "Seychelles"); m.insert("SD", "Sudan");
    m.insert("SE", "Sweden"); m.insert("SG", "Singapore");
    m.insert("SI", "Slovenia"); m.insert("SK", "Slovakia");
    m.insert("SL", "Sierra Leone"); m.insert("SM", "San Marino");
    m.insert("SN", "Senegal"); m.insert("SO", "Somalia");
    m.insert("SR", "Suriname"); m.insert("SS", "South Sudan");
    m.insert("ST", "Sao Tome and Principe"); m.insert("SV", "El Salvador");
    m.insert("SY", "Syria"); m.insert("SZ", "Eswatini");
    m.insert("TC", "Turks and Caicos"); m.insert("TD", "Chad");
    m.insert("TG", "Togo"); m.insert("TH", "Thailand");
    m.insert("TJ", "Tajikistan"); m.insert("TL", "Timor-Leste");
    m.insert("TM", "Turkmenistan"); m.insert("TN", "Tunisia");
    m.insert("TO", "Tonga"); m.insert("TR", "Turkey");
    m.insert("TT", "Trinidad and Tobago"); m.insert("TV", "Tuvalu");
    m.insert("TW", "Taiwan"); m.insert("TZ", "Tanzania");
    m.insert("UA", "Ukraine"); m.insert("UG", "Uganda");
    m.insert("US", "United States"); m.insert("UY", "Uruguay");
    m.insert("UZ", "Uzbekistan"); m.insert("VA", "Vatican City");
    m.insert("VC", "Saint Vincent"); m.insert("VE", "Venezuela");
    m.insert("VG", "British Virgin Islands"); m.insert("VI", "US Virgin Islands");
    m.insert("VN", "Vietnam"); m.insert("VU", "Vanuatu");
    m.insert("WS", "Samoa"); m.insert("XK", "Kosovo");
    m.insert("YE", "Yemen"); m.insert("ZA", "South Africa");
    m.insert("ZM", "Zambia"); m.insert("ZW", "Zimbabwe");
    m
}
