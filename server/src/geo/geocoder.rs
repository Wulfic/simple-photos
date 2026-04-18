//! Offline reverse geocoder using a spatial grid index.
//!
//! Loads GeoNames cities500.txt (~200k cities) into a grid-based spatial
//! index. Each grid cell covers ~1° × 1° and contains all cities whose
//! coordinates fall within. Lookups check the cell + its 8 neighbours and
//! find the nearest city by haversine distance.
//!
//! No external crates needed — just a HashMap of grid cells.

use std::collections::HashMap;
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
    state: Option<String>,
    country: String,
    country_code: String,
}

/// Grid-based spatial index for fast nearest-city lookup.
pub struct ReverseGeocoder {
    /// Grid cells keyed by (lat_bucket, lon_bucket). Each bucket is 1° × 1°.
    grid: HashMap<(i32, i32), Vec<City>>,
    city_count: usize,
}

impl ReverseGeocoder {
    /// Load the GeoNames cities500.txt dataset from disk.
    ///
    /// Format: tab-separated, columns:
    /// 0: geonameid, 1: name, 2: asciiname, 3: alternatenames,
    /// 4: latitude, 5: longitude, 6: feature class, 7: feature code,
    /// 8: country code, 9: cc2, 10: admin1, ..., 17: population
    pub fn load(path: &Path) -> Result<Self, String> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read GeoNames file '{}': {}", path.display(), e))?;

        let mut grid: HashMap<(i32, i32), Vec<City>> = HashMap::new();
        let mut count = 0usize;

        for line in contents.lines() {
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

            let city = City {
                name: fields[1].to_string(),
                lat,
                lon,
                state: if fields[10].is_empty() { None } else { Some(fields[10].to_string()) },
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

        tracing::info!(cities = count, buckets = grid.len(), "Loaded GeoNames dataset");
        Ok(Self { grid, city_count: count })
    }

    /// Create an empty geocoder (for when geo is disabled or dataset isn't available).
    pub fn empty() -> Self {
        Self { grid: HashMap::new(), city_count: 0 }
    }

    /// Whether this geocoder has any data loaded.
    pub fn is_loaded(&self) -> bool {
        self.city_count > 0
    }

    /// Number of cities loaded.
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
                        if best.as_ref().map_or(true, |(_, d)| dist < *d) {
                            best = Some((city, dist));
                        }
                    }
                }
            }
        }

        best.map(|(city, _)| GeoLocation {
            city: city.name.clone(),
            state: city.state.clone(),
            country: city.country.clone(),
            country_code: city.country_code.clone(),
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
