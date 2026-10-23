// Cost harness for the falling-ragdoll scene (row PX45).
//
// One scene, one source file, two engines: float Box3D and the Q48.16
// fixed-point fork. Each build compiles this file with its own headers and its
// own copy of shared/determinism.c, so the ragdoll grid comes from the donor,
// not from this file.
//
// Scene identity between the two engines is reported, not assumed: the dynamic
// body count at creation (`awake_at_create`), and the tick at which the scene
// falls asleep (`probe_sleep_step`, untimed). Two engines whose ragdolls differ
// in body count or sleep tick are not timing the same work.
//
// Build with -DCOST_FIXED=1 for the fixed-point engine: only the time-step
// expression and b3World_Step's argument type differ between the two APIs.

#include "determinism.h"

#include "box3d/box3d.h"

#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined( _WIN64 )
#include <windows.h>
static double NowMs( void )
{
	static LARGE_INTEGER frequency;
	static bool haveFrequency = false;
	if ( haveFrequency == false )
	{
		QueryPerformanceFrequency( &frequency );
		haveFrequency = true;
	}
	LARGE_INTEGER counter;
	QueryPerformanceCounter( &counter );
	return (double)counter.QuadPart * 1000.0 / (double)frequency.QuadPart;
}

// Process CPU time (user + kernel). Other sessions on this shared PC steal wall
// time but not this, so it is the number the ratio is computed from.
static double NowCpuMs( void )
{
	FILETIME creation, exit, kernel, user;
	GetProcessTimes( GetCurrentProcess(), &creation, &exit, &kernel, &user );
	ULARGE_INTEGER k, u;
	k.LowPart = kernel.dwLowDateTime;
	k.HighPart = kernel.dwHighDateTime;
	u.LowPart = user.dwLowDateTime;
	u.HighPart = user.dwHighDateTime;
	return (double)( k.QuadPart + u.QuadPart ) / 10000.0;
}
#else
#include <time.h>
static double NowMs( void )
{
	struct timespec ts;
	clock_gettime( CLOCK_MONOTONIC, &ts );
	return (double)ts.tv_sec * 1000.0 + (double)ts.tv_nsec / 1e6;
}

static double NowCpuMs( void )
{
	return (double)clock() * 1000.0 / (double)CLOCKS_PER_SEC;
}
#endif

static void StepOnce( b3WorldId worldId )
{
#if defined( COST_FIXED )
	b3Fixed timeStep = b3FixDiv( B3_FIX( 1.0f ), B3_FIX( 60.0f ) );
	b3World_Step( worldId, timeStep, 4 );
#else
	float timeStep = 1.0f / 60.0f;
	b3World_Step( worldId, timeStep, 4 );
#endif
}

// Fresh scene, stepped until every body sleeps. Untimed; only reports how long
// the active phase of this scene is, in ticks.
static int ProbeSleepStep( void )
{
	b3WorldDef worldDef = b3DefaultWorldDef();
	b3WorldId worldId = b3CreateWorld( &worldDef );
	FallingRagdollData data = CreateFallingRagdolls( worldId );

	int steps = 0;
	while ( b3World_GetAwakeBodyCount( worldId ) > 0 && steps < 2000 )
	{
		StepOnce( worldId );
		++steps;
	}

	DestroyFallingRagdolls( &data );
	b3DestroyWorld( worldId );
	return steps;
}

int main( int argc, char** argv )
{
	const char* label = COST_LABEL;
	int steps = 500;
	int reps = 5;

	for ( int i = 1; i < argc; ++i )
	{
		if ( strcmp( argv[i], "--steps" ) == 0 && i + 1 < argc )
		{
			steps = atoi( argv[++i] );
		}
		else if ( strcmp( argv[i], "--reps" ) == 0 && i + 1 < argc )
		{
			reps = atoi( argv[++i] );
		}
		else if ( strcmp( argv[i], "--label" ) == 0 && i + 1 < argc )
		{
			label = argv[++i];
		}
		else
		{
			fprintf( stderr, "bad argument %s\n", argv[i] );
			return 2;
		}
	}

	b3WorldDef worldDef = b3DefaultWorldDef();
	int awakeAtCreate = -1;

	for ( int rep = 0; rep < reps; ++rep )
	{
		b3WorldId worldId = b3CreateWorld( &worldDef );
		FallingRagdollData data = CreateFallingRagdolls( worldId );

		if ( awakeAtCreate < 0 )
		{
			awakeAtCreate = b3World_GetAwakeBodyCount( worldId );
		}

		double start = NowMs();
		double startCpu = NowCpuMs();
		for ( int step = 0; step < steps; ++step )
		{
			StepOnce( worldId );
		}
		double elapsed = NowMs() - start;
		double cpuElapsed = NowCpuMs() - startCpu;

		// Untimed: how much of the timed window still had awake bodies. A window
		// that is mostly asleep measures almost nothing, in either engine.
		int awakeAfter = b3World_GetAwakeBodyCount( worldId );

		printf( "engine=%s rep=%d steps=%d cpu_ms=%.3f wall_ms=%.3f awake_at_create=%d awake_after=%d\n", label, rep,
				steps, cpuElapsed, elapsed, awakeAtCreate, awakeAfter );

		DestroyFallingRagdolls( &data );
		b3DestroyWorld( worldId );
	}

	printf( "engine=%s probe sleep_step=%d awake_at_create=%d\n", label, ProbeSleepStep(), awakeAtCreate );

	return 0;
}
