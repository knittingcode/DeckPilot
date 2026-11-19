use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::*;

const USE: Thresholds = Thresholds {
    warning: 70,
    critical: 90,
};
const FREE: Thresholds = Thresholds {
    warning: 20,
    critical: 10,
};
const GIB: u64 = 1 << 30;

#[test]
fn levels_of_use_and_free_space() {
    assert_eq!(USE.of_use(0), Level::Ok);
    assert_eq!(USE.of_use(69), Level::Ok);
    assert_eq!(USE.of_use(70), Level::Warning);
    assert_eq!(USE.of_use(89), Level::Warning);
    assert_eq!(USE.of_use(90), Level::Critical);
    assert_eq!(USE.of_use(100), Level::Critical);
    assert_eq!(FREE.of_free(50), Level::Ok);
    assert_eq!(FREE.of_free(21), Level::Ok);
    assert_eq!(FREE.of_free(20), Level::Warning);
    assert_eq!(FREE.of_free(11), Level::Warning);
    assert_eq!(FREE.of_free(10), Level::Critical);
    assert_eq!(FREE.of_free(0), Level::Critical);
    // Thresholds in the wrong order give the stronger level.
    let swapped = Thresholds {
        warning: 90,
        critical: 70,
    };
    assert_eq!(swapped.of_use(80), Level::Critical);
    assert_eq!(
        [Level::Ok, Level::Warning, Level::Critical].map(Level::indicator),
        [Indicator::Ok, Indicator::Warning, Indicator::Error]
    );
}

#[test]
fn percents_are_whole_and_bounded() {
    assert_eq!(percent(1, 3), Some(33));
    assert_eq!(percent(2, 3), Some(67));
    assert_eq!(percent(5, 0), None);
    assert_eq!(percent(7, 5), Some(100));
    assert_eq!(cpu_percent(37.4), Some(37));
    assert_eq!(cpu_percent(-1.0), Some(0));
    assert_eq!(cpu_percent(130.0), Some(100));
    assert_eq!(cpu_percent(f32::NAN), None);
}

#[test]
fn throughput_units() {
    let cases = [
        (0.0, "0 B/s", "0B"),
        (999.0, "999 B/s", "999B"),
        (1_234.0, "1.2 kB/s", "1.2K"),
        (9_960.0, "10 kB/s", "10K"),
        (56_000.0, "56 kB/s", "56K"),
        (999_960.0, "1.0 MB/s", "1.0M"),
        (1_240_000.0, "1.2 MB/s", "1.2M"),
        (123_400_000.0, "123 MB/s", "123M"),
        (2_500_000_000.0, "2.5 GB/s", "2.5G"),
        (-5.0, "0 B/s", "0B"),
        (f64::NAN, "0 B/s", "0B"),
    ];
    for (value, long, short) in cases {
        assert_eq!(rate(value), long, "{value}");
        assert_eq!(compact_rate(value), short, "{value}");
        assert!(short.chars().count() <= 4, "{short}");
    }
    assert_eq!(gib(16 * GIB), "16.0");
    assert_eq!(gib(GIB * 99 / 10), "9.9");
}

#[test]
fn cpu_keys() {
    let report = cpu_report(37, &[10, 20, 37], USE);
    assert_eq!(report.message, "CPU 37%");
    assert_eq!(
        report.data,
        [
            ("usagePercent", "37".to_owned()),
            ("level", "ok".to_owned())
        ]
    );
    assert_eq!(
        report.state,
        ButtonState::new()
            .with_detail("37%")
            .with_indicator(Indicator::Ok)
            .with_progress(37)
            .with_series([10, 20, 37])
    );
    // One value is no chart yet.
    let busy = cpu_report(95, &[95], USE);
    assert_eq!(busy.state.series, None);
    assert_eq!(busy.state.indicator, Some(Indicator::Error));
    assert_eq!(busy.data[1].1, "critical");
}

#[test]
fn memory_keys() {
    let report = memory_report(
        Memory {
            used: GIB * 99 / 10,
            total: 16 * GIB,
        },
        USE,
    )
    .unwrap();
    assert_eq!(report.message, "memory 62% used (9.9 of 16.0 GiB)");
    assert_eq!(
        report.state,
        ButtonState::new()
            .with_detail("9.9/16 GiB")
            .with_indicator(Indicator::Ok)
            .with_progress(62)
    );
    assert_eq!(report.data[0], ("usedPercent", "62".to_owned()));
    assert_eq!(short_gib(32 * GIB), "32");
    assert_eq!(short_gib(GIB * 15 / 2), "7.5");
    assert_eq!(report.data[2], ("totalBytes", (16 * GIB).to_string()));
    let full = memory_report(
        Memory {
            used: 15 * GIB,
            total: 16 * GIB,
        },
        USE,
    )
    .unwrap();
    assert_eq!(full.state.indicator, Some(Indicator::Error));
    let none = memory_report(Memory { used: 0, total: 0 }, USE).unwrap_err();
    assert_eq!(none.code, codes::UNAVAILABLE);
}

fn interface(name: &str, received: f64, transmitted: f64, loopback: bool) -> InterfaceRate {
    InterfaceRate {
        name: name.into(),
        received,
        transmitted,
        loopback,
    }
}

#[test]
fn network_keys() {
    let rates = [
        interface("en0", 1_000_000.0, 50_000.0, false),
        interface("en1", 240_000.0, 6_000.0, false),
        interface("lo0", 9_000_000.0, 9_000_000.0, true),
    ];
    // All but loopback, added up.
    let all = network_report(&rates, None).unwrap();
    assert_eq!(all.message, "in 1.2 MB/s, out 56 kB/s");
    assert_eq!(
        all.data,
        [
            ("receivedPerSecond", "1240000".to_owned()),
            ("transmittedPerSecond", "56000".to_owned()),
            ("interfaces", "en0,en1".to_owned()),
        ]
    );
    assert_eq!(
        all.state,
        ButtonState::new().with_detail("↓ 1.2 MB/s\n↑ 56 kB/s")
    );
    assert_eq!(network_report(&rates, Some("  ")).unwrap(), all);
    // One interface by name, case ignored; loopback when asked for.
    let one = network_report(&rates, Some("EN1")).unwrap();
    assert_eq!(one.message, "EN1: in 240 kB/s, out 6.0 kB/s");
    assert_eq!(one.data[2].1, "en1");
    assert_eq!(
        network_report(&rates, Some("lo0")).unwrap().data[0].1,
        "9000000"
    );
    let missing = network_report(&rates, Some("wlan9")).unwrap_err();
    assert_eq!(missing.code, codes::NOT_FOUND);
    // No interface at all is no traffic.
    let quiet = network_report(&[], None).unwrap();
    assert_eq!(quiet.state.detail.as_deref(), Some("↓ 0 B/s\n↑ 0 B/s"));
    // Two short lines that fit the narrowest key; the title stays.
    let busy = network_report(&[interface("eth0", 999_000.0, 999_000.0, false)], None).unwrap();
    assert_eq!(busy.state.label, None);
    let detail = busy.state.detail.unwrap();
    assert_eq!(detail.lines().count(), 2);
    for text in detail.lines() {
        assert!(text.chars().count() <= 10, "{text}");
    }
}

#[test]
fn loopback_interfaces() {
    let v4 = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let v6 = IpAddr::V6(Ipv6Addr::LOCALHOST);
    let lan = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));
    assert!(is_loopback("lo0", &[v4, v6]));
    assert!(!is_loopback("en0", &[lan, v6]));
    // Without addresses, by name.
    assert!(is_loopback("lo", &[]));
    assert!(is_loopback("lo0", &[]));
    assert!(is_loopback("Loopback Pseudo-Interface 1", &[]));
    assert!(!is_loopback("eth0", &[]));
    assert!(!is_loopback("local-bridge", &[]));
}

fn volume(mount: &str, total: u64, available: u64) -> Volume {
    Volume {
        mount_point: PathBuf::from(mount),
        total,
        available,
    }
}

#[test]
fn the_volume_of_a_folder() {
    let volumes = [
        volume("/", 500 * GIB, 100 * GIB),
        volume("/home", 1000 * GIB, 400 * GIB),
        volume("/home/data", 0, 0),
        volume("/media/usb", 64 * GIB, 60 * GIB),
    ];
    let on = |path: &str| {
        volume_for(Path::new(path), &volumes)
            .map(|v| v.mount_point.display().to_string())
            .unwrap_or_default()
    };
    assert_eq!(on("/home/ada/projects"), "/home");
    assert_eq!(on("/home"), "/home");
    // A prefix of a name is not a parent folder.
    assert_eq!(on("/homework"), "/");
    // A volume without a size is skipped.
    assert_eq!(on("/home/data/x"), "/home");
    assert_eq!(on("/media/usb/"), "/media/usb");
    assert_eq!(on("/etc"), "/");
    // Windows: verbatim paths and drive letters.
    let windows = [
        volume(r"C:\", 500 * GIB, 50 * GIB),
        volume(r"D:\", GIB, GIB),
    ];
    let found = volume_for(Path::new(r"\\?\C:\Users\ada"), &windows).unwrap();
    assert_eq!(found.mount_point, PathBuf::from(r"C:\"));
    assert!(volume_for(Path::new("relative"), &windows).is_none());
}

#[test]
fn disk_keys() {
    let report = disk_report(&volume("/", 500 * GIB, 225 * GIB), FREE).unwrap();
    assert_eq!(report.message, "45% free on / (225.0 of 500.0 GiB)");
    assert_eq!(
        report.state,
        ButtonState::new()
            .with_detail("55% used")
            .with_indicator(Indicator::Ok)
            .with_progress(55)
    );
    assert_eq!(report.data[3], ("mountPoint", "/".to_owned()));
    assert_eq!(report.data[4], ("level", "ok".to_owned()));
    let low = disk_report(&volume("/", 100 * GIB, 15 * GIB), FREE).unwrap();
    assert_eq!(low.state.indicator, Some(Indicator::Warning));
    let full = disk_report(&volume("/", 100 * GIB, 5 * GIB), FREE).unwrap();
    assert_eq!(full.state.indicator, Some(Indicator::Error));
    let unknown = disk_report(&volume("/", 0, 0), FREE).unwrap_err();
    assert_eq!(unknown.code, codes::UNAVAILABLE);
}

#[test]
fn texts_fit_the_key() {
    // The values a key shows under its title are short: the renderer
    // shrinks a value a little, then cuts it, on the narrowest key (72 px,
    // where "12.4/32 GiB" still fits whole).
    let cpu = cpu_report(100, &[], USE).state;
    let memory = memory_report(
        Memory {
            used: 128 * GIB - 1,
            total: 128 * GIB,
        },
        USE,
    )
    .unwrap()
    .state;
    let disk = disk_report(&volume("/", 100, 0), FREE).unwrap().state;
    for (state, longest) in [(cpu, 4), (memory, 13), (disk, 9)] {
        let detail = state.detail.unwrap();
        assert!(detail.chars().count() <= longest, "{detail}");
        assert_eq!(state.label, None, "the key keeps its title");
    }
}
