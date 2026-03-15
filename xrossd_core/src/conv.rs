pub fn db_to_float(db: f64) -> f32 {
    let res = if db <= -90.0 { 0.0 }
    else if db < -60.0 { (db + 90.0) / 480.0 }
    else if db < -30.0 { (db + 70.0) / 160.0 }
    else if db < -10.0 { (db + 50.0) / 80.0 }
    else if db <= 10.0 { (db + 30.0) / 40.0 }
    else { 1.0 };
    res as f32
}

pub fn float_to_db(float: f32) -> f64 {
    let f = float as f64;
    if f >= 0.5 {
        40.0 * f - 30.0
    } else if f >= 0.25 {
        80.0 * f - 50.0
    } else if f >= 0.0625 {
        160.0 * f - 70.0
    } else if f > 0.0 {
        480.0 * f - 90.0
    } else {
        f64::NEG_INFINITY
    }
}
