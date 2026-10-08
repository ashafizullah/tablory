CREATE TYPE mood AS ENUM ('happy', 'sad');
CREATE TABLE customers (
  id serial PRIMARY KEY,
  name text NOT NULL,
  email varchar(200) UNIQUE,
  vip boolean NOT NULL DEFAULT false,
  balance numeric(12, 2) DEFAULT 0,
  big bigint,
  feeling mood,
  tags text[],
  meta jsonb,
  avatar bytea,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE orders (
  id serial PRIMARY KEY,
  customer_id int NOT NULL REFERENCES customers(id),
  total real,
  placed date DEFAULT current_date
);
CREATE INDEX orders_customer_idx ON orders(customer_id);
CREATE TABLE audit_log (at timestamptz DEFAULT now(), message text);
CREATE VIEW vip_customers AS SELECT * FROM customers WHERE vip;
INSERT INTO customers (name, email, vip, balance, big, feeling, tags, meta, avatar)
SELECT 'Customer ' || g, 'c' || g || '@example.com', g % 7 = 0, g * 1.5, 9007199254740993 + g,
       CASE WHEN g % 2 = 0 THEN 'happy'::mood ELSE 'sad'::mood END,
       ARRAY['a', 'b'], jsonb_build_object('n', g), '\xdeadbeef'
FROM generate_series(1, 5000) g;
INSERT INTO orders (customer_id, total) SELECT (g % 5000) + 1, g * 0.25 FROM generate_series(1, 20000) g;
INSERT INTO audit_log (message) VALUES ('no primary key here');
CREATE SCHEMA reporting;
CREATE TABLE reporting."Weird ""Name""" (id int PRIMARY KEY, "Mixed Case" text);
INSERT INTO reporting."Weird ""Name""" VALUES (1, 'it''s quoted');
