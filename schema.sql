pragma journal_mode = wal;
pragma foreign_keys = on;

CREATE TABLE company_batches(
    id integer primary key,
    batch_name text unique not null,
    description text,
    --unix time stamps for both of these dates
    start_date integer not null default(strftime('%s', 'now')),
    end_date integer
) strict;

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
    cancelled integer not null default(0),
    primary key(batch_id, ssn)
) strict;
CREATE INDEX batch_relat_ssn on ssn_batch_relat(ssn);

create table envelopes(
    id integer primary key,
    guid text unique,
    ssn integer not null references acct_data(ssn) on delete cascade,
    status text,
    api_err text,
    fname text,
    mname text,
    lname text,
    --Use ISO8601 for dob
    dob text,
    addr1 text,
    addr2 text,
    city text,
    sate text,
    zip text,
    email text,
    phone text,
    spouse_fname text,
    spouse_mname text,
    spouse_lname text,
    spouse_email text,
    date_created integer not null default(strftime('%s', 'now')),
    last_update integer,
    pdf blob
) strict;
create index envelope_ssn on envelopes(ssn);
create unique index envelopes_restrict_active on envelopes(ssn) where status is null or status not in ('completed', 'declined', 'voided', 'spouse_beneficiary');

CREATE TABLE beneficiaries(
    gid text primary key references envelopes(gid) on delete cascade,
    --primary/contingent
    type text,
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
    gid text primary key references envelopes(gid) on delete cascade,
    name text,
    --use iso8601
    dob text
) strict;
create index author_gid on authorized_users(gid);