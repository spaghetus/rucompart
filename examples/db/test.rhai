fn filter(key, value) {
    key.starts_with("item-")
}

fn map(key, value) {
    value
}

fn reduce (left, right) {
    #{
        "list": (left["list"]) + (right["list"]),
        "number": (left["number"]) + (right["number"]),
    }
}