//! Process-wide clock, black box. This binary shares one global source, so only one test in it
//! may install a source.

mod install;
