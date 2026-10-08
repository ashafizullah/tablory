CREATE TABLE customers (
  id int AUTO_INCREMENT PRIMARY KEY,
  name varchar(100) NOT NULL,
  email varchar(200) UNIQUE,
  vip tinyint(1) NOT NULL DEFAULT 0,
  balance decimal(12, 2) DEFAULT 0,
  big bigint unsigned,
  meta json,
  avatar blob,
  created_at datetime DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE orders (
  id int AUTO_INCREMENT PRIMARY KEY,
  customer_id int NOT NULL,
  total double,
  FOREIGN KEY (customer_id) REFERENCES customers(id)
);
CREATE TABLE audit_log (at datetime, message text);
CREATE VIEW vip_customers AS SELECT * FROM customers WHERE vip = 1;
INSERT INTO customers (name, email, vip, balance, big, meta, avatar)
WITH RECURSIVE seq(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < 1000)
SELECT CONCAT('Customer ', n), CONCAT('c', n, '@example.com'), n % 7 = 0, n * 1.5,
       18446744073709551000 + (n % 600), JSON_OBJECT('n', n), UNHEX('DEADBEEF') FROM seq;
INSERT INTO orders (customer_id, total) SELECT id, id * 0.25 FROM customers;
INSERT INTO audit_log VALUES (NOW(), 'no primary key here');
