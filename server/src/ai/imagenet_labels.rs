//! ImageNet 1000-class labels for MobileNetV2 classification.
//!
//! These labels correspond to the output indices of the MobileNetV2-12 model
//! trained on ImageNet (ILSVRC2012). Each index maps to a human-readable class.
//!
//! The label list itself lives in `server/data/imagenet_labels.txt` (one
//! class per line, ordered by index 0..1000) and is bundled into the binary
//! at compile time via `include_str!`.  This keeps the Rust source small and
//! lets ops swap label sets without recompiling.

use std::sync::LazyLock;

/// Raw bundled label data (one class per line).
const IMAGENET_LABELS_RAW: &str = include_str!("imagenet_labels.txt");

/// The 1000 ImageNet class labels, indexed by model output class ID.
pub static IMAGENET_LABELS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let labels: Vec<&'static str> = IMAGENET_LABELS_RAW
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    debug_assert_eq!(
        labels.len(),
        1000,
        "imagenet_labels.txt must contain exactly 1000 non-empty lines, got {}",
        labels.len()
    );
    labels
});

/// Map an ImageNet class index to a broader category useful for photo tagging.
///
/// Returns `Some(category)` for classes relevant to personal photo libraries,
/// `None` for obscure/irrelevant classes that would create noisy tags.
pub fn label_category(index: usize) -> Option<&'static str> {
    match index {
        // Fish
        0..=6 => Some("fish"),
        389..=397 => Some("fish"),

        // Birds (rooster, hen, ostrich, other birds)
        7..=24 => Some("bird"),
        80..=100 => Some("bird"),

        // Reptiles & amphibians
        25..=68 => Some("reptile"),

        // Arachnids (spiders, scorpions, ticks)
        69..=79 => Some("insect"),

        // Australian/exotic mammals
        101..=106 => Some("animal"),

        // Marine invertebrates
        107..=126 => Some("sea creature"),

        // Misc small animals (stork, flamingo, pelican, penguin, etc.)
        127..=150 => Some("bird"),

        // Dog breeds (all 120 breeds)
        151..=268 => Some("dog"),

        // Wolves, foxes, wild canines
        269..=280 => Some("wild animal"),

        // Domestic cats
        281..=285 => Some("cat"),

        // Wild cats (cougar, lynx, leopard, snow leopard, jaguar, lion, tiger, cheetah)
        286..=293 => Some("big cat"),

        // Bears
        294..=297 => Some("bear"),

        // Mongoose, meerkat
        298..=299 => Some("animal"),

        // Beetles, insects, butterflies
        300..=326 => Some("insect"),

        // Starfish, sea urchin, sea cucumber
        327..=329 => Some("sea creature"),

        // Small mammals (rabbit, hare, hamster, porcupine, squirrel, etc.)
        330..=338 => Some("animal"),

        // Horse
        339 => Some("horse"),

        // Large mammals (zebra, pig, hippo, ox, bison, camel, llama, etc.)
        340..=364 => Some("animal"),

        // Primates
        365..=384 => Some("animal"),

        // Elephants
        385..=386 => Some("elephant"),

        // Pandas
        387..=388 => Some("animal"),

        // Objects start at 398
        // (vehicles, furniture, electronics etc. handled by name matching below)

        // Food & produce
        924..=969 => Some("food"),

        // Natural landscapes
        970..=980 => Some("landscape"),

        // People
        981..=983 => Some("person"),

        // Plants & flowers
        984..=987 => Some("plant"),

        // Nuts, seeds
        988..=990 => Some("plant"),

        // Fungi
        991..=997 => Some("mushroom"),

        // Fallback: check by name for objects in 398-923 range
        _ => label_category_by_name(index),
    }
}

/// Fallback category mapping by label name for the large objects range (398-923).
fn label_category_by_name(index: usize) -> Option<&'static str> {
    if index >= IMAGENET_LABELS.len() {
        return None;
    }
    let label = IMAGENET_LABELS[index];

    // Vehicles
    if matches!(
        label,
        "ambulance"
            | "airliner"
            | "airship"
            | "amphibian"
            | "beach wagon"
            | "cab"
            | "canoe"
            | "car wheel"
            | "catamaran"
            | "convertible"
            | "fire engine"
            | "forklift"
            | "garbage truck"
            | "go-kart"
            | "golfcart"
            | "gondola"
            | "horse cart"
            | "jeep"
            | "lifeboat"
            | "limousine"
            | "liner"
            | "minibus"
            | "minivan"
            | "moped"
            | "motor scooter"
            | "mountain bike"
            | "moving van"
            | "oxcart"
            | "pickup"
            | "police van"
            | "racer"
            | "recreational vehicle"
            | "rickshaw"
            | "school bus"
            | "snowmobile"
            | "snowplow"
            | "speedboat"
            | "sports car"
            | "streetcar"
            | "submarine"
            | "tank"
            | "tow truck"
            | "tractor"
            | "trailer truck"
            | "tricycle"
            | "trolleybus"
            | "unicycle"
            | "warplane"
            | "yawl"
    ) {
        return Some("vehicle");
    }

    // Buildings & architecture
    if matches!(
        label,
        "bakery"
            | "barn"
            | "barbershop"
            | "beacon"
            | "boathouse"
            | "bookshop"
            | "castle"
            | "church"
            | "cinema"
            | "cliff dwelling"
            | "dam"
            | "dome"
            | "greenhouse"
            | "grocery store"
            | "library"
            | "lumbermill"
            | "monastery"
            | "mosque"
            | "palace"
            | "planetarium"
            | "prison"
            | "restaurant"
            | "shoe shop"
            | "stupa"
            | "tobacco shop"
            | "toyshop"
            | "triumphal arch"
            | "yurt"
    ) {
        return Some("building");
    }

    // Musical instruments
    if matches!(
        label,
        "accordion"
            | "acoustic guitar"
            | "banjo"
            | "bassoon"
            | "cello"
            | "chime"
            | "cornet"
            | "drum"
            | "drumstick"
            | "electric guitar"
            | "flute"
            | "French horn"
            | "gong"
            | "grand piano"
            | "harmonica"
            | "harp"
            | "maraca"
            | "marimba"
            | "oboe"
            | "organ"
            | "panpipe"
            | "sax"
            | "steel drum"
            | "trombone"
            | "upright"
            | "violin"
    ) {
        return Some("music");
    }

    // Electronics / tech
    if matches!(
        label,
        "CD player"
            | "cassette player"
            | "cellular telephone"
            | "computer keyboard"
            | "desktop computer"
            | "digital clock"
            | "digital watch"
            | "iPod"
            | "joystick"
            | "laptop"
            | "loudspeaker"
            | "microphone"
            | "modem"
            | "monitor"
            | "mouse"
            | "notebook"
            | "printer"
            | "projector"
            | "remote control"
            | "television"
            | "typewriter keyboard"
    ) {
        return Some("electronics");
    }

    // Sports equipment
    if matches!(
        label,
        "balance beam"
            | "barbell"
            | "baseball"
            | "basketball"
            | "croquet ball"
            | "dumbbell"
            | "football helmet"
            | "golf ball"
            | "horizontal bar"
            | "parallel bars"
            | "ping-pong ball"
            | "puck"
            | "punching bag"
            | "racket"
            | "rugby ball"
            | "scoreboard"
            | "ski"
            | "ski mask"
            | "soccer ball"
            | "tennis ball"
            | "volleyball"
    ) {
        return Some("sports");
    }

    // Kitchen / cooking / dining
    if matches!(
        label,
        "cocktail shaker"
            | "coffee mug"
            | "coffeepot"
            | "Crock Pot"
            | "dishwasher"
            | "Dutch oven"
            | "espresso maker"
            | "frying pan"
            | "goblet"
            | "ladle"
            | "measuring cup"
            | "microwave"
            | "mixing bowl"
            | "pitcher"
            | "plate rack"
            | "pot"
            | "refrigerator"
            | "rotisserie"
            | "soup bowl"
            | "spatula"
            | "strainer"
            | "teapot"
            | "toaster"
            | "waffle iron"
            | "water jug"
            | "wine bottle"
            | "wok"
            | "wooden spoon"
    ) {
        return Some("kitchen");
    }

    // Clothing & fashion
    if matches!(
        label,
        "abaya"
            | "academic gown"
            | "apron"
            | "bathing cap"
            | "bikini"
            | "bonnet"
            | "bow tie"
            | "brassiere"
            | "cardigan"
            | "cloak"
            | "cowboy boot"
            | "cowboy hat"
            | "clog"
            | "diaper"
            | "feather boa"
            | "fur coat"
            | "gown"
            | "jean"
            | "jersey"
            | "kimono"
            | "Loafer"
            | "maillot"
            | "miniskirt"
            | "mitten"
            | "necklace"
            | "overskirt"
            | "pajama"
            | "poncho"
            | "purse"
            | "running shoe"
            | "sandal"
            | "sarong"
            | "ski mask"
            | "sombrero"
            | "stole"
            | "suit"
            | "sunglasses"
            | "sweatshirt"
            | "swimming trunks"
            | "trench coat"
            | "vestment"
            | "wig"
            | "Windsor tie"
    ) {
        return Some("clothing");
    }

    // Furniture
    if matches!(
        label,
        "bassinet"
            | "bookcase"
            | "china cabinet"
            | "chiffonier"
            | "cradle"
            | "crib"
            | "desk"
            | "dining table"
            | "entertainment center"
            | "filing cabinet"
            | "folding chair"
            | "four-poster"
            | "park bench"
            | "rocking chair"
            | "studio couch"
            | "table lamp"
            | "throne"
            | "wardrobe"
    ) {
        return Some("furniture");
    }

    // Outdoor / recreation
    if matches!(
        label,
        "backpack"
            | "balloon"
            | "binoculars"
            | "candle"
            | "carousel"
            | "Christmas stocking"
            | "flagpole"
            | "fountain"
            | "jack-o'-lantern"
            | "lawn mower"
            | "mountain tent"
            | "parachute"
            | "picket fence"
            | "pier"
            | "shopping cart"
            | "sleeping bag"
            | "snowboard"
            | "stone wall"
            | "sundial"
            | "suspension bridge"
            | "swing"
            | "umbrella"
            | "vase"
    ) {
        return Some("outdoor");
    }

    // Toys & games
    if matches!(
        label,
        "jigsaw puzzle"
            | "pinwheel"
            | "teddy"
            | "comic book"
            | "crossword puzzle"
    ) {
        return Some("toy");
    }

    None
}
