//! ImageNet 1000-class labels for MobileNetV2 classification.
//!
//! These labels correspond to the output indices of the MobileNetV2-12 model
//! trained on ImageNet (ILSVRC2012). Each index maps to a human-readable class.

/// The 1000 ImageNet class labels, indexed by model output class ID.
pub const IMAGENET_LABELS: [&str; 1000] = [
    "tench",
    "goldfish",
    "great white shark",
    "tiger shark",
    "hammerhead",
    "electric ray",
    "stingray",
    "cock",
    "hen",
    "ostrich",
    "brambling",
    "goldfinch",
    "house finch",
    "junco",
    "indigo bunting",
    "robin",
    "bulbul",
    "jay",
    "magpie",
    "chickadee",
    "water ouzel",
    "kite",
    "bald eagle",
    "vulture",
    "great grey owl",
    "European fire salamander",
    "common newt",
    "eft",
    "spotted salamander",
    "axolotl",
    "bullfrog",
    "tree frog",
    "tailed frog",
    "loggerhead",
    "leatherback turtle",
    "mud turtle",
    "terrapin",
    "box turtle",
    "banded gecko",
    "common iguana",
    "American chameleon",
    "whiptail",
    "agama",
    "frilled lizard",
    "alligator lizard",
    "Gila monster",
    "green lizard",
    "African chameleon",
    "Komodo dragon",
    "African crocodile",
    "American alligator",
    "triceratops",
    "thunder snake",
    "ringneck snake",
    "hognose snake",
    "green snake",
    "king snake",
    "garter snake",
    "water snake",
    "vine snake",
    "night snake",
    "boa constrictor",
    "rock python",
    "Indian cobra",
    "green mamba",
    "sea snake",
    "horned viper",
    "diamondback",
    "sidewinder",
    "trilobite",
    "harvestman",
    "scorpion",
    "black and gold garden spider",
    "barn spider",
    "garden spider",
    "black widow",
    "tarantula",
    "wolf spider",
    "tick",
    "centipede",
    "black grouse",
    "ptarmigan",
    "ruffed grouse",
    "prairie chicken",
    "peacock",
    "quail",
    "partridge",
    "African grey",
    "macaw",
    "sulphur-crested cockatoo",
    "lorikeet",
    "coucal",
    "bee eater",
    "hornbill",
    "hummingbird",
    "jacamar",
    "toucan",
    "drake",
    "red-breasted merganser",
    "goose",
    "black swan",
    "tusker",
    "echidna",
    "platypus",
    "wallaby",
    "koala",
    "wombat",
    "jellyfish",
    "sea anemone",
    "brain coral",
    "flatworm",
    "nematode",
    "conch",
    "snail",
    "slug",
    "sea slug",
    "chiton",
    "chambered nautilus",
    "Dungeness crab",
    "rock crab",
    "fiddler crab",
    "king crab",
    "American lobster",
    "spiny lobster",
    "crayfish",
    "hermit crab",
    "isopod",
    "white stork",
    "black stork",
    "spoonbill",
    "flamingo",
    "little blue heron",
    "American egret",
    "bittern",
    "crane",
    "limpkin",
    "European gallinule",
    "American coot",
    "bustard",
    "ruddy turnstone",
    "red-backed sandpiper",
    "redshank",
    "dowitcher",
    "oystercatcher",
    "pelican",
    "king penguin",
    "albatross",
    "grey whale",
    "killer whale",
    "dugong",
    "sea lion",
    "Chihuahua",
    "Japanese spaniel",
    "Maltese dog",
    "Pekinese",
    "Shih-Tzu",
    "Blenheim spaniel",
    "papillon",
    "toy terrier",
    "Rhodesian ridgeback",
    "Afghan hound",
    "basset",
    "beagle",
    "bloodhound",
    "bluetick",
    "black-and-tan coonhound",
    "Walker hound",
    "English foxhound",
    "redbone",
    "borzoi",
    "Irish wolfhound",
    "Italian greyhound",
    "whippet",
    "Ibizan hound",
    "Norwegian elkhound",
    "otterhound",
    "Saluki",
    "Scottish deerhound",
    "Weimaraner",
    "Staffordshire bullterrier",
    "American Staffordshire terrier",
    "Bedlington terrier",
    "Border terrier",
    "Kerry blue terrier",
    "Irish terrier",
    "Norfolk terrier",
    "Norwich terrier",
    "Yorkshire terrier",
    "wire-haired fox terrier",
    "Lakeland terrier",
    "Sealyham terrier",
    "Airedale",
    "cairn",
    "Australian terrier",
    "Dandie Dinmont",
    "Boston bull",
    "miniature schnauzer",
    "giant schnauzer",
    "standard schnauzer",
    "Scotch terrier",
    "Tibetan terrier",
    "silky terrier",
    "soft-coated wheaten terrier",
    "West Highland white terrier",
    "Lhasa",
    "flat-coated retriever",
    "curly-coated retriever",
    "golden retriever",
    "Labrador retriever",
    "Chesapeake Bay retriever",
    "German short-haired pointer",
    "vizsla",
    "English setter",
    "Irish setter",
    "Gordon setter",
    "Brittany spaniel",
    "clumber",
    "English springer",
    "Welsh springer spaniel",
    "cocker spaniel",
    "Sussex spaniel",
    "Irish water spaniel",
    "kuvasz",
    "schipperke",
    "groenendael",
    "malinois",
    "briard",
    "kelpie",
    "komondor",
    "Old English sheepdog",
    "Shetland sheepdog",
    "collie",
    "Border collie",
    "Bouvier des Flandres",
    "Rottweiler",
    "German shepherd",
    "Doberman",
    "miniature pinscher",
    "Greater Swiss Mountain dog",
    "Bernese mountain dog",
    "Appenzeller",
    "EntleBucher",
    "boxer",
    "bull mastiff",
    "Tibetan mastiff",
    "French bulldog",
    "Great Dane",
    "Saint Bernard",
    "Eskimo dog",
    "malamute",
    "Siberian husky",
    "dalmatian",
    "affenpinscher",
    "basenji",
    "pug",
    "Leonberg",
    "Newfoundland",
    "Great Pyrenees",
    "Samoyed",
    "Pomeranian",
    "chow",
    "keeshond",
    "Brabancon griffon",
    "Pembroke",
    "Cardigan",
    "toy poodle",
    "miniature poodle",
    "standard poodle",
    "Mexican hairless",
    "timber wolf",
    "white wolf",
    "red wolf",
    "coyote",
    "dingo",
    "dhole",
    "African hunting dog",
    "hyena",
    "red fox",
    "kit fox",
    "Arctic fox",
    "grey fox",
    "tabby",
    "tiger cat",
    "Persian cat",
    "Siamese cat",
    "Egyptian cat",
    "cougar",
    "lynx",
    "leopard",
    "snow leopard",
    "jaguar",
    "lion",
    "tiger",
    "cheetah",
    "brown bear",
    "American black bear",
    "ice bear",
    "sloth bear",
    "mongoose",
    "meerkat",
    "tiger beetle",
    "ladybug",
    "ground beetle",
    "long-horned beetle",
    "leaf beetle",
    "dung beetle",
    "rhinoceros beetle",
    "weevil",
    "fly",
    "bee",
    "ant",
    "grasshopper",
    "cricket",
    "walking stick",
    "cockroach",
    "mantis",
    "cicada",
    "leafhopper",
    "lacewing",
    "dragonfly",
    "damselfly",
    "admiral",
    "ringlet",
    "monarch",
    "cabbage butterfly",
    "sulphur butterfly",
    "lycaenid",
    "starfish",
    "sea urchin",
    "sea cucumber",
    "wood rabbit",
    "hare",
    "Angora",
    "hamster",
    "porcupine",
    "fox squirrel",
    "marmot",
    "beaver",
    "guinea pig",
    "sorrel",
    "zebra",
    "hog",
    "wild boar",
    "warthog",
    "hippopotamus",
    "ox",
    "water buffalo",
    "bison",
    "ram",
    "bighorn",
    "ibex",
    "hartebeest",
    "impala",
    "gazelle",
    "Arabian camel",
    "llama",
    "weasel",
    "mink",
    "polecat",
    "black-footed ferret",
    "otter",
    "skunk",
    "badger",
    "armadillo",
    "three-toed sloth",
    "orangutan",
    "gorilla",
    "chimpanzee",
    "gibbon",
    "siamang",
    "guenon",
    "patas",
    "baboon",
    "macaque",
    "langur",
    "colobus",
    "proboscis monkey",
    "marmoset",
    "capuchin",
    "howler monkey",
    "titi",
    "spider monkey",
    "squirrel monkey",
    "Madagascar cat",
    "indri",
    "Indian elephant",
    "African elephant",
    "lesser panda",
    "giant panda",
    "barracouta",
    "eel",
    "coho",
    "rock beauty",
    "anemone fish",
    "sturgeon",
    "gar",
    "lionfish",
    "puffer",
    "abacus",
    "abaya",
    "academic gown",
    "accordion",
    "acoustic guitar",
    "aircraft carrier",
    "airliner",
    "airship",
    "altar",
    "ambulance",
    "amphibian",
    "analog clock",
    "apiary",
    "apron",
    "ashcan",
    "assault rifle",
    "backpack",
    "bakery",
    "balance beam",
    "balloon",
    "ballpoint",
    "Band Aid",
    "banjo",
    "bannister",
    "barbell",
    "barber chair",
    "barbershop",
    "barn",
    "barometer",
    "barrel",
    "barrow",
    "baseball",
    "basketball",
    "bassinet",
    "bassoon",
    "bathing cap",
    "bath towel",
    "bathtub",
    "beach wagon",
    "beacon",
    "beaker",
    "bearskin",
    "beer bottle",
    "beer glass",
    "bell cote",
    "bib",
    "bicycle-built-for-two",
    "bikini",
    "binder",
    "binoculars",
    "birdhouse",
    "boathouse",
    "bobsled",
    "bolo tie",
    "bonnet",
    "bookcase",
    "bookshop",
    "bottlecap",
    "bow",
    "bow tie",
    "brass",
    "brassiere",
    "breakwater",
    "breastplate",
    "broom",
    "bucket",
    "buckle",
    "bulletproof vest",
    "bullet train",
    "butcher shop",
    "cab",
    "caldron",
    "candle",
    "cannon",
    "canoe",
    "can opener",
    "cardigan",
    "car mirror",
    "carousel",
    "carpenter's kit",
    "carton",
    "car wheel",
    "cash machine",
    "cassette",
    "cassette player",
    "castle",
    "catamaran",
    "CD player",
    "cello",
    "cellular telephone",
    "chain",
    "chainlink fence",
    "chain mail",
    "chain saw",
    "chest",
    "chiffonier",
    "chime",
    "china cabinet",
    "Christmas stocking",
    "church",
    "cinema",
    "cleaver",
    "cliff dwelling",
    "cloak",
    "clog",
    "cocktail shaker",
    "coffee mug",
    "coffeepot",
    "coil",
    "combination lock",
    "computer keyboard",
    "confectionery",
    "container ship",
    "convertible",
    "corkscrew",
    "cornet",
    "cowboy boot",
    "cowboy hat",
    "cradle",
    "crane",
    "crash helmet",
    "crate",
    "crib",
    "Crock Pot",
    "croquet ball",
    "crutch",
    "cuirass",
    "dam",
    "desk",
    "desktop computer",
    "dial telephone",
    "diaper",
    "digital clock",
    "digital watch",
    "dining table",
    "dishrag",
    "dishwasher",
    "disk brake",
    "dock",
    "dogsled",
    "dome",
    "doormat",
    "drilling platform",
    "drum",
    "drumstick",
    "dumbbell",
    "Dutch oven",
    "electric fan",
    "electric guitar",
    "electric locomotive",
    "entertainment center",
    "envelope",
    "espresso maker",
    "face powder",
    "feather boa",
    "file",
    "fireboat",
    "fire engine",
    "fire screen",
    "flagpole",
    "flute",
    "folding chair",
    "football helmet",
    "forklift",
    "fountain",
    "fountain pen",
    "four-poster",
    "freight car",
    "French horn",
    "frying pan",
    "fur coat",
    "garbage truck",
    "gasmask",
    "gas pump",
    "goblet",
    "go-kart",
    "golf ball",
    "golfcart",
    "gondola",
    "gong",
    "gown",
    "grand piano",
    "greenhouse",
    "grille",
    "grocery store",
    "guillotine",
    "hair slide",
    "hair spray",
    "half track",
    "hammer",
    "hamper",
    "hand blower",
    "hand-held computer",
    "handkerchief",
    "hard disc",
    "harmonica",
    "harp",
    "harvester",
    "hatchet",
    "holster",
    "home theater",
    "honeycomb",
    "hook",
    "hoopskirt",
    "horizontal bar",
    "horse cart",
    "hourglass",
    "iPod",
    "iron",
    "jack-o'-lantern",
    "jean",
    "jeep",
    "jersey",
    "jigsaw puzzle",
    "jinrikisha",
    "joystick",
    "kimono",
    "knee pad",
    "knot",
    "lab coat",
    "ladle",
    "lampshade",
    "laptop",
    "lawn mower",
    "lens cap",
    "letter opener",
    "library",
    "lifeboat",
    "lighter",
    "limousine",
    "liner",
    "lipstick",
    "Loafer",
    "lotion",
    "loudspeaker",
    "loupe",
    "lumbermill",
    "magnetic compass",
    "mailbag",
    "mailbox",
    "maillot",
    "maillot",
    "manhole cover",
    "maraca",
    "marimba",
    "mask",
    "matchstick",
    "maypole",
    "maze",
    "measuring cup",
    "medicine chest",
    "megalith",
    "microphone",
    "microwave",
    "military uniform",
    "milk can",
    "minibus",
    "miniskirt",
    "minivan",
    "missile",
    "mitten",
    "mixing bowl",
    "mobile home",
    "Model T",
    "modem",
    "monastery",
    "monitor",
    "moped",
    "mortar",
    "mortarboard",
    "mosque",
    "mosquito net",
    "motor scooter",
    "mountain bike",
    "mountain tent",
    "mouse",
    "mousetrap",
    "moving van",
    "muzzle",
    "nail",
    "neck brace",
    "necklace",
    "nipple",
    "notebook",
    "obelisk",
    "oboe",
    "ocarina",
    "odometer",
    "oil filter",
    "organ",
    "oscilloscope",
    "overskirt",
    "oxcart",
    "oxygen mask",
    "packet",
    "paddle",
    "paddlewheel",
    "padlock",
    "paintbrush",
    "pajama",
    "palace",
    "panpipe",
    "paper towel",
    "parachute",
    "parallel bars",
    "park bench",
    "parking meter",
    "passenger car",
    "patio",
    "pay-phone",
    "pedestal",
    "pencil box",
    "pencil sharpener",
    "perfume",
    "Petri dish",
    "photocopier",
    "pick",
    "pickelhaube",
    "picket fence",
    "pickup",
    "pier",
    "piggy bank",
    "pill bottle",
    "pillow",
    "ping-pong ball",
    "pinwheel",
    "pirate",
    "pitcher",
    "plane",
    "planetarium",
    "plastic bag",
    "plate rack",
    "plow",
    "plunger",
    "Polaroid camera",
    "pole",
    "police van",
    "poncho",
    "pool table",
    "pop bottle",
    "pot",
    "potter's wheel",
    "power drill",
    "prayer rug",
    "printer",
    "prison",
    "projectile",
    "projector",
    "puck",
    "punching bag",
    "purse",
    "quill",
    "quilt",
    "racer",
    "racket",
    "radiator",
    "radio",
    "radio telescope",
    "rain barrel",
    "recreational vehicle",
    "reel",
    "reflex camera",
    "refrigerator",
    "remote control",
    "restaurant",
    "revolver",
    "rifle",
    "rocking chair",
    "rotisserie",
    "rubber eraser",
    "rugby ball",
    "rule",
    "running shoe",
    "safe",
    "safety pin",
    "saltshaker",
    "sandal",
    "sarong",
    "sax",
    "scabbard",
    "scale",
    "school bus",
    "schooner",
    "scoreboard",
    "screen",
    "screw",
    "screwdriver",
    "seat belt",
    "sewing machine",
    "shield",
    "shoe shop",
    "shoji",
    "shopping basket",
    "shopping cart",
    "shovel",
    "shower cap",
    "shower curtain",
    "ski",
    "ski mask",
    "sleeping bag",
    "slide rule",
    "sliding door",
    "slot",
    "snorkel",
    "snowmobile",
    "snowplow",
    "soap dispenser",
    "soccer ball",
    "sock",
    "solar dish",
    "sombrero",
    "soup bowl",
    "space bar",
    "space heater",
    "space shuttle",
    "spatula",
    "speedboat",
    "spider web",
    "spindle",
    "sports car",
    "spotlight",
    "stage",
    "steam locomotive",
    "steel arch bridge",
    "steel drum",
    "stethoscope",
    "stole",
    "stone wall",
    "stopwatch",
    "stove",
    "strainer",
    "streetcar",
    "stretcher",
    "studio couch",
    "stupa",
    "submarine",
    "suit",
    "sundial",
    "sunglass",
    "sunglasses",
    "sunscreen",
    "suspension bridge",
    "swab",
    "sweatshirt",
    "swimming trunks",
    "swing",
    "switch",
    "syringe",
    "table lamp",
    "tank",
    "tape player",
    "teapot",
    "teddy",
    "television",
    "tennis ball",
    "thatch",
    "theater curtain",
    "thimble",
    "thresher",
    "throne",
    "tile roof",
    "toaster",
    "tobacco shop",
    "toilet seat",
    "torch",
    "totem pole",
    "tow truck",
    "toyshop",
    "tractor",
    "trailer truck",
    "tray",
    "trench coat",
    "tricycle",
    "trimaran",
    "tripod",
    "triumphal arch",
    "trolleybus",
    "trombone",
    "tub",
    "turnstile",
    "typewriter keyboard",
    "umbrella",
    "unicycle",
    "upright",
    "vacuum",
    "vase",
    "vault",
    "velvet",
    "vending machine",
    "vestment",
    "viaduct",
    "violin",
    "volleyball",
    "waffle iron",
    "wall clock",
    "wallet",
    "wardrobe",
    "warplane",
    "washbasin",
    "washer",
    "water bottle",
    "water jug",
    "water tower",
    "whiskey jug",
    "whistle",
    "wig",
    "window screen",
    "window shade",
    "Windsor tie",
    "wine bottle",
    "wing",
    "wok",
    "wooden spoon",
    "wool",
    "worm fence",
    "wreck",
    "yawl",
    "yurt",
    "web site",
    "comic book",
    "crossword puzzle",
    "street sign",
    "traffic light",
    "book jacket",
    "menu",
    "plate",
    "guacamole",
    "consomme",
    "hot pot",
    "trifle",
    "ice cream",
    "ice lolly",
    "French loaf",
    "bagel",
    "pretzel",
    "cheeseburger",
    "hotdog",
    "mashed potato",
    "head cabbage",
    "broccoli",
    "cauliflower",
    "zucchini",
    "spaghetti squash",
    "acorn squash",
    "butternut squash",
    "cucumber",
    "artichoke",
    "bell pepper",
    "cardoon",
    "mushroom",
    "Granny Smith",
    "strawberry",
    "orange",
    "lemon",
    "fig",
    "pineapple",
    "banana",
    "jackfruit",
    "custard apple",
    "pomegranate",
    "hay",
    "carbonara",
    "chocolate sauce",
    "dough",
    "meat loaf",
    "pizza",
    "potpie",
    "burrito",
    "red wine",
    "espresso",
    "cup",
    "eggnog",
    "alp",
    "bubble",
    "cliff",
    "coral reef",
    "geyser",
    "lakeside",
    "promontory",
    "sandbar",
    "seashore",
    "valley",
    "volcano",
    "ballplayer",
    "groom",
    "scuba diver",
    "rapeseed",
    "daisy",
    "yellow lady's slipper",
    "corn",
    "acorn",
    "hip",
    "buckeye",
    "coral fungus",
    "agaric",
    "gyromitra",
    "stinkhorn",
    "earthstar",
    "hen-of-the-woods",
    "bolete",
    "ear",
    "toilet tissue",
];

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
