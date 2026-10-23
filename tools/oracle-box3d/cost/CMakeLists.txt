cmake_minimum_required(VERSION 3.22)
project(ragdoll_cost C)

# Both engine headers and the donor's own scene sources come from -DBOX3D_SRC;
# -DBOX3D_LIB is the static library that donor tree already built.
if(NOT BOX3D_SRC)
	message(FATAL_ERROR "pass -DBOX3D_SRC=<donor checkout>")
endif()
if(NOT BOX3D_LIB)
	message(FATAL_ERROR "pass -DBOX3D_LIB=<built box3d library>")
endif()
if(NOT BOX3D_LABEL)
	set(BOX3D_LABEL "engine")
endif()

add_library(donor_box3d STATIC IMPORTED)
set_target_properties(donor_box3d PROPERTIES IMPORTED_LOCATION "${BOX3D_LIB}")

add_executable(ragdoll_cost
	ragdoll_cost.c
	"${BOX3D_SRC}/shared/determinism.c"
	"${BOX3D_SRC}/shared/human.c"
	"${BOX3D_SRC}/shared/utils.c"
)

target_include_directories(ragdoll_cost PRIVATE
	"${BOX3D_SRC}/include"
	"${BOX3D_SRC}/shared"
)

# fixed3d vendors the Q48.16 library it is built on; Box3D has no such directory.
if(EXISTS "${BOX3D_SRC}/extern/fixed/include")
	target_include_directories(ragdoll_cost PRIVATE "${BOX3D_SRC}/extern/fixed/include")
endif()

# COST_LABEL names the engine in the output; BOX3D_FIXED selects the fixed-point
# step call (b3World_Step takes b3Fixed there, float in Box3D).
target_compile_definitions(ragdoll_cost PRIVATE
	COST_LABEL="${BOX3D_LABEL}"
	$<$<BOOL:${BOX3D_FIXED}>:COST_FIXED=1>
)

target_link_libraries(ragdoll_cost PRIVATE donor_box3d)

if(MSVC)
	target_compile_options(ragdoll_cost PRIVATE /W3)
endif()
