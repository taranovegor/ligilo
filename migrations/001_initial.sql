CREATE TABLE IF NOT EXISTS urls (
    id         BIGSERIAL     PRIMARY KEY,
    code       VARCHAR(5)    NOT NULL,
    url        VARCHAR(2048) NOT NULL,
    created_at TIMESTAMPTZ   NOT NULL DEFAULT now(),
    CONSTRAINT urls_code_unique UNIQUE (code)
);

CREATE INDEX IF NOT EXISTS idx_urls_code ON urls(code);
