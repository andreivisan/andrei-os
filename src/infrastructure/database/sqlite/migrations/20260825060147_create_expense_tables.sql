-- create categories table
CREATE TABLE categories (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    category_group TEXT NOT NULL
);

-- create expenses table
CREATE TABLE expenses (
    id INTEGER PRIMARY KEY,
    amount INTEGER NOT NULL,
    currency TEXT NOT NULL,
    category_id INTEGER NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (category_id) REFERENCES categories(id)
);