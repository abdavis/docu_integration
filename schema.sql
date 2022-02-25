pragma journal_mode = wal;
pragma foreign_keys = on;
CREATE TABLE company_batches(
    id integer primary key,
    batch_name text not null,
    description text,
    --unix time stamps for both of these dates
    start_date integer not null default(strftime('%s', 'now')),
    end_date integer
) strict;
CREATE UNIQUE INDEX company_batches_unique_name ON company_batches(batch_name) WHERE end_date IS NULL;
CREATE INDEX company_batches_end ON company_batches(end_date);
CREATE TABLE acct_data(
    ssn integer primary key,
    primary_acct integer,
    info_codes text,
    created_account integer,
    host_err text
) strict;
CREATE TABLE ssn_batch_relat(
    batch_id integer not null references company_batches(id) on delete cascade,
    ssn integer not null references acct_data(ssn) on delete cascade,
    --boolean: 0/1
    ignore_error integer not null default(FALSE),
    primary key(batch_id, ssn)
) strict;
CREATE INDEX batch_relat_ssn on ssn_batch_relat(ssn);
create table envelopes(
    id integer primary key,
    gid text unique,
    ssn integer not null references acct_data(ssn) on delete cascade,
    status text,
    void_reason text,
    api_err text,
    fname text NOT NULL,
    mname text,
    lname text NOT NULL,
    --Use ISO8601 for dob
    dob text NOT NULL,
    addr1 text NOT NULL,
    addr2 text,
    city text NOT NULL,
    state text NOT NULL,
    zip text NOT NULL,
    email text NOT NULL,
    phone text NOT NULL,
    spouse_fname text,
    spouse_mname text,
    spouse_lname text,
    spouse_email text,
    date_created integer not null default(strftime('%s', 'now')) -- make sure all spouse fields are empty, or fname, lname, and email are populated
    CHECK(
        (
            spouse_fname IS NULL
            AND spouse_mname IS NULL
            AND spouse_lname IS NULL
            AND spouse_email IS NULL
        )
        OR (
            spouse_fname IS NOT NULL
            AND spouse_lname IS NOT NULL
            AND spouse_email IS NOT NULL
        )
    )
) strict;
create index envelope_ssn on envelopes(ssn);
create unique index envelopes_restrict_active on envelopes(ssn)
where status is null
    or status not in ('completed', 'declined', 'voided', 'cancelled');
CREATE TABLE beneficiaries(
    gid text not null references envelopes(gid) on delete cascade,
    --primary/contingent
    type text CHECK(type in ('primary', 'contingent')),
    name text,
    address text,
    city_state_zip text,
    --use iso8601
    dob text,
    relationship text,
    ssn integer,
    percent integer
) strict;
create index benefic_gid on beneficiaries(gid);
CREATE TABLE authorized_users(
    gid text not null references envelopes(gid) on delete cascade,
    name text,
    --use iso8601
    dob text
) strict;
create index author_gid on authorized_users(gid);
create table pdf(
    gid text primary key not null references envelopes(gid) on delete cascade,
    complete_pdf blob
) strict;