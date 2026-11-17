// The bundled DuckDB fmt version selects a removed MSVC checked iterator
// whenever _SECURE_SCL is defined. Load the standard headers first, then
// disable that obsolete fmt-specific branch for this translation unit.
#include <vector>
#ifdef _SECURE_SCL
#undef _SECURE_SCL
#endif
